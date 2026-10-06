// Mixed saturation: how the twin desks, ash-rust and Ash in Elixir, treat a small request
// while expensive ones fill them up. See README.md.
//
//   node examples/supportdesk/bench/saturation.ts --fixture /tmp/fixture.json
//   node examples/supportdesk/bench/saturation.ts --target astro      # CPU-bound, in memory
//
// Two streams of requests arrive at once, each at a fixed rate whatever the desk's answers:
//
//   cheap  one ticket by id, a steady `--cheap-rate` a second, as a viewer;
//   heavy  the 250 newest tickets of an org with their relationships and aggregates, ramped
//          from nothing to well past what the desk can do.
//
// A desk that holds up under load keeps the cheap stream quick while the heavy one outgrows
// it, and turns the excess away; one that doesn't lets the cheap requests wait behind the
// heavy ones, and its tail grows, until requests time out. The stream stops, and the cheap
// one is watched until it recovers.
//
// Latency is counted from when each request was due, so a desk that falls behind is charged
// for the wait. A request unanswered after `--timeout` is given up on, as a client would, and
// counts as a failure at that latency.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { cpus, totalmem } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";

import { binaries, desks, extraEnv, fresh, psql, seedTemplate, stop, usage, type Desk, type Side } from "./desks.ts";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { astroBinaries, astroDesks, probeAstro, seedTickets, startAstro } from "./astro.ts";
import { rng, scenarios, world, type World } from "./scenarios.ts";
import { median } from "./stats.ts";
import type { Job, JobResult } from "./worker.ts";

// Options ----------------------------------------------------------------------------

const args = process.argv.slice(2);
const flag = (name: string, fallback?: string): string | undefined => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : fallback;
};
const quick = args.includes("--quick");
const options = {
  /** `supportdesk`: the desks over Postgres, the heavy request a large page of tickets with
   *  their relationships. `astro`: the in-memory astro-helpdesk twins, no database, the
   *  heavy request a sort, filter and count over every ticket: CPU and nothing else. */
  target: flag("--target", "supportdesk")!,
  /** `astro`: tickets each desk holds. */
  tickets: Number(flag("--tickets", "5000")),
  fixture: flag("--fixture"),
  /** Instead of the ramp: each pool configuration, one 30 s test at each of `rates`, the same
   *  heavy rate to every desk (`supportdesk` target only). */
  matrix: args.includes("--matrix"),
  pools: flag("--pools", "20,40")!.split(",").map(Number),
  /** Instead of each desk's own pool policy: the same wait limit for both, in milliseconds,
   *  each at the first of `pools` connections. Rust's is `POOL_WAIT_MS`. Ecto has no hard limit:
   *  under overload it drops what has waited past twice its `queue_target`, so Elixir is given
   *  a `queue_target` of half the limit, and a short `queue_interval` so that engages at once. */
  limits: flag("--limits")?.split(",").map(Number),
  rates: flag("--rates", "800,1600")!.split(",").map(Number),
  /** The Postgres container, to sample its CPU. */
  pgContainer: flag("--pg-container", "ash-bench-pg")!,
  pg: flag("--pg", "postgres://postgres:postgres@127.0.0.1:55434")!,
  threads: Number(flag("--threads", "6")),
  /** Of the driver's threads, those that send the cheap stream; the rest send the heavy one. */
  cheapThreads: Number(flag("--cheap-threads", "2")),
  cheapRate: Number(flag("--cheap-rate", quick ? "200" : "500")),
  /** The heavy rate at each step, as a multiple of a capacity (see `basis`). */
  levels: flag("--levels", quick ? "0.5,1,2" : flag("--target", "supportdesk") === "astro" ? "0.25,0.5,1,2,4,8" : "0.25,0.5,1,1.5,2,3,4")!.split(",").map(Number),
  /** `common`: every desk is offered the same heavy rate, multiples of the slower desk's
   *  capacity, so what's compared is how each handles the same load. `own`: multiples of
   *  its own, so what's compared is how each handles being at 100%, 200% of what it can do. */
  basis: flag("--basis", "common")!,
  warmup: Number(flag("--warmup", quick ? "1000" : args.includes("--matrix") ? "3000" : "2000")),
  window: Number(flag("--window", args.includes("--matrix") ? (quick ? "5000" : "30000") : quick ? "4000" : "10000")),
  /** A pause between steps, the cheap stream running, for what's queued to drain. */
  settle: Number(flag("--settle", quick ? "1500" : "4000")),
  recovery: Number(flag("--recovery", quick ? "6000" : "15000")),
  calibrateClients: Number(flag("--calibrate-clients", "24")),
  calibrateWindow: Number(flag("--calibrate-window", quick ? "3000" : "6000")),
  timeout: Number(flag("--timeout", "10000")),
  /** Heavy requests the driver holds unanswered at most, so it can't be the one to run out.
   *  A desk that queues rather than refuses holds that many at its worst, so this bounds how
   *  long its queue can grow: past it, requests are counted as not sent, never as errors. */
  maxInFlight: Number(flag("--max-in-flight", flag("--target", "supportdesk") === "astro" ? "500" : "3000")),
  reps: Number(flag("--reps", quick || args.includes("--matrix") ? "1" : "2")),
  /** Stop ramping a desk whose server passes this resident memory, so a runaway can't take the machine. */
  maxRssGiB: Number(flag("--max-rss-gib", "20")),
};
if (!["supportdesk", "astro"].includes(options.target)) throw new Error("--target is supportdesk or astro");
const astro = options.target === "astro";
if (!astro && !options.fixture) {
  console.error("usage: node saturation.ts --fixture fixture.json [--target supportdesk|astro] [--quick] [--reps 2] [--basis common|own] [--levels 0.5,1,2]");
  process.exit(2);
}
if (!["common", "own"].includes(options.basis)) throw new Error("--basis is common or own");

// The run's directory and manifest ---------------------------------------------------

const here = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../../../", import.meta.url));
const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const dir = `${here}runs/saturation-${stamp}`;
mkdirSync(dir, { recursive: true });
const sh = (cmd: string, cwd = repo) => spawnSync("bash", ["-c", cmd], { cwd, encoding: "utf8" }).stdout.trim();
const hash = (path: string) => createHash("sha256").update(readFileSync(path)).digest("hex").slice(0, 16);
const manifest = {
  started: new Date().toISOString(),
  target: options.target,
  git: { rev: sh("git rev-parse HEAD"), dirty: sh("git status --porcelain").length > 0 },
  binaries: astro
    ? {
        rust: { path: astroBinaries.rust, sha256: hash(astroBinaries.rust) },
        elixir: { release: astroBinaries.elixir, version: sh("cat examples/elixir/astro-helpdesk/_build/prod/rel/astro_helpdesk/releases/start_erl.data") },
      }
    : {
        rust: { path: binaries.rust, sha256: hash(binaries.rust) },
        elixir: { release: binaries.elixir, version: sh("cat examples/elixir/supportdesk/_build/prod/rel/supportdesk/releases/start_erl.data") },
      },
  machine: {
    cpu: cpus()[0].model,
    cores: cpus().length,
    memoryGiB: Math.round(totalmem() / 2 ** 30),
    os: sh("uname -sr"),
    loadAverageAtStart: sh("uptime").split("load averages:")[1]?.trim(),
  },
  ...(astro ? {} : { postgres: psql(options.pg, "SELECT version()"), fixture: { path: options.fixture, sha256: hash(options.fixture!) } }),
  node: process.version,
  options,
};
writeFileSync(`${dir}/manifest.json`, JSON.stringify(manifest, null, 2));
const record = (entry: object) => appendFileSync(`${dir}/results.jsonl`, JSON.stringify(entry) + "\n");
const log = (side: Side) => `${dir}/${side}.log`;

const w: World = astro ? { orgs: [] } : world(JSON.parse(readFileSync(options.fixture!, "utf8")));
const both = astro ? astroDesks() : desks();
const sides: Side[] = ["rust", "elixir"];
const order = (rep: number): Side[] => (rep % 2 === 0 ? sides : [...sides].reverse());
const cheap = scenarios.find((s) => s.name === (astro ? "sat-cpu-cheap" : "sat-cheap"))!;
const heavy = scenarios.find((s) => s.name === (astro ? "sat-cpu-heavy" : "sat-heavy"))!;

/** Starts `side` as it is to be measured: the supportdesk desk on a fresh copy of the seeded
 *  database, the astro desk restarted and seeded with the tickets. */
const freshDesk = async (side: Side) => {
  if (!astro) return fresh(options.pg, both[side], log(side));
  await startAstro(both[side], log(side));
  await seedTickets(both[side], options.tickets);
};

// Load threads -----------------------------------------------------------------------

const threads = Array.from(
  { length: options.threads },
  (_, index) => new Worker(new URL("./worker.ts", import.meta.url), { workerData: { world: w, index } }),
);
const cheapThreads = threads.slice(0, options.cheapThreads);
const heavyThreads = threads.slice(options.cheapThreads);
const runJob = (thread: Worker, job: Job) =>
  new Promise<JobResult>((resolve, reject) => {
    const failed = (error: Error) => {
      thread.off("message", done);
      reject(error);
    };
    const done = (result: JobResult) => {
      thread.off("error", failed);
      resolve(result);
    };
    thread.once("message", done);
    thread.once("error", failed);
    thread.postMessage(job);
  });

// Measuring --------------------------------------------------------------------------

const quantile = (sorted: number[], q: number): number | null => (sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(q * sorted.length))] : null);
const round = (v: number | null, digits = 2): number | null => (v === null ? null : Math.round(v * 10 ** digits) / 10 ** digits);

type Stream = {
  /** Requests due in the window. */
  due: number;
  /** Of those, how many were answered successfully, failed, and timed out. */
  ok: number;
  failed: number;
  timeouts: number;
  /** Latency (ms) of the successes. */
  p50: number | null;
  p90: number | null;
  p99: number | null;
  /** Of every request due, the failures counted at the latency they gave up at: what a
   *  client waited. */
  p99All: number | null;
  max: number | null;
  /** Due while the driver held its limit, not sent. */
  dropped: number;
  unfinished: number;
  late: number;
};
type Window = {
  /** The heavy requests due a second. */
  offered: number;
  cheap: Stream;
  heavy: Stream | null;
  /** Heavy requests answered successfully, finished inside the window, a second. */
  goodput: number;
  server: { cores: number | null; maxRssMiB: number | null };
  /** Postgres while the window ran (matrix runs): connections in use, and the container's CPU. */
  db?: { active: number | null; connections: number | null; containerCpuPct: number | null } | null;
  /** The cheap stream by second of the window: [p99 of everything due in it, failures]. */
  seconds?: Array<{ p50: number | null; p99: number | null; failed: number }>;
};

const stream = (results: JobResult[]): Stream => {
  const ok = results.flatMap((r) => Array.from(r.latencies)).sort((a, b) => a - b);
  const failures = results.flatMap((r) => Array.from(r.failures));
  const all = [...ok, ...failures].sort((a, b) => a - b);
  const sum = (f: (r: JobResult) => number) => results.reduce((t, r) => t + f(r), 0);
  return {
    due: all.length + sum((r) => r.dropped ?? 0),
    ok: ok.length,
    failed: failures.length,
    timeouts: sum((r) => r.timeouts ?? 0),
    p50: round(quantile(ok, 0.5)),
    p90: round(quantile(ok, 0.9)),
    p99: round(quantile(ok, 0.99)),
    p99All: round(quantile(all, 0.99)),
    max: round(all.length ? all[all.length - 1] : null),
    dropped: sum((r) => r.dropped ?? 0),
    unfinished: sum((r) => r.unfinished),
    late: sum((r) => r.late),
  };
};

const exec = promisify(execFile);
const mean = (values: number[]): number | null => (values.length ? Math.round((values.reduce((t, v) => t + v, 0) / values.length) * 10) / 10 : null);

/** Until `until`: how many connections Postgres has in use by this desk's database, and how
 *  busy its container is, to tell a pool that's too small from a database that's out of CPU. */
const sampleDb = async (desk: Desk, until: number) => {
  const active: number[] = [];
  const connections: number[] = [];
  const cpu: number[] = [];
  while (Date.now() < until) {
    try {
      const sql = "select count(*) filter (where state = 'active'), count(*) from pg_stat_activity where datname = current_database() and pid <> pg_backend_pid()";
      const { stdout } = await exec("psql", [`${options.pg}/${desk.db}`, "-qAt", "-c", sql]);
      const [a, c] = stdout.trim().split("|").map(Number);
      active.push(a);
      connections.push(c);
    } catch {}
    try {
      const { stdout } = await exec("docker", ["stats", "--no-stream", "--format", "{{.CPUPerc}}", options.pgContainer]);
      cpu.push(parseFloat(stdout));
    } catch {}
  }
  return { active: mean(active), connections: mean(connections), containerCpuPct: mean(cpu) };
};

/** One window: the cheap stream, and the heavy one at `heavyRate` a second (none: 0). */
const run = async (desk: Desk, heavyRate: number, window = options.window, seconds = false): Promise<Window> => {
  const start = Date.now() + 50;
  const from = start + options.warmup;
  const until = from + window;
  const base = { transport: "graphql" as const, base: desk.base, seed: 4242, start, from, until, timeoutMs: options.timeout };
  const cheapJobs = cheapThreads.map((_, i) => ({
    ...base,
    scenario: cheap.name,
    rate: options.cheapRate / cheapThreads.length,
    phase: i / cheapThreads.length,
    maxInFlight: 500,
    timeline: seconds,
  }));
  const heavyJobs = heavyRate
    ? heavyThreads.map((_, i) => ({
        ...base,
        scenario: heavy.name,
        rate: heavyRate / heavyThreads.length,
        phase: i / heavyThreads.length,
        maxInFlight: Math.ceil(options.maxInFlight / heavyThreads.length),
      }))
    : [];
  let maxRss = 0;
  let before: ReturnType<typeof usage>;
  let after: ReturnType<typeof usage>;
  const sampling = (async () => {
    while (Date.now() < from) await sleep(10);
    before = usage(desk.pid);
    while (Date.now() < until) {
      maxRss = Math.max(maxRss, usage(desk.pid)?.rss ?? 0);
      await sleep(250);
    }
    after = usage(desk.pid);
  })();
  const dbSampling = options.matrix
    ? (async () => {
        while (Date.now() < from) await sleep(10);
        return sampleDb(desk, until);
      })()
    : Promise.resolve(null);
  const [cheapResults, heavyResults] = await Promise.all([
    Promise.all(cheapJobs.map((job, i) => runJob(cheapThreads[i], job))),
    Promise.all(heavyJobs.map((job, i) => runJob(heavyThreads[i], job))),
  ]);
  await sampling;
  const db = await dbSampling;
  const timeline = cheapResults.flatMap((r) => r.timeline ?? []);
  return {
    offered: heavyRate,
    cheap: stream(cheapResults),
    heavy: heavyRate ? stream(heavyResults) : null,
    goodput: round(heavyResults.reduce((t, r) => t + r.completed, 0) / (window / 1000), 1) ?? 0,
    db,
    server: {
      cores: before && after ? round((after.cpu - before.cpu) / (window / 1000)) : null,
      maxRssMiB: maxRss ? Math.round(maxRss / 1024) : null,
    },
    ...(seconds
      ? {
          seconds: Array.from({ length: Math.ceil(window / 1000) }, (_, s) => {
            const rows = timeline.filter(([second]) => second === s);
            const sorted = rows.map(([, ms]) => ms).sort((a, b) => a - b);
            return { p50: round(quantile(sorted, 0.5)), p99: round(quantile(sorted, 0.99)), failed: rows.filter(([, , ok]) => !ok).length };
          }),
        }
      : {}),
  };
};

/** What the desk can do of the heavy request alone: its throughput with clients asking
 *  again as soon as answered. */
const capacity = async (desk: Desk): Promise<number> => {
  const start = Date.now() + 50;
  const from = start + 1500;
  const until = from + options.calibrateWindow;
  const n = threads.length;
  const results = await Promise.all(
    threads.map((thread, i) =>
      runJob(thread, {
        scenario: heavy.name,
        transport: "graphql",
        base: desk.base,
        seed: 99,
        start,
        from,
        until,
        clients: Math.floor(options.calibrateClients / n) + (i < options.calibrateClients % n ? 1 : 0),
      }),
    ),
  );
  return Math.round((results.reduce((t, r) => t + r.completed, 0) / (options.calibrateWindow / 1000)) * 10) / 10;
};

// Comparing answers ------------------------------------------------------------------

const canonical = (value: unknown): string =>
  JSON.stringify(value, (_, v) =>
    v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b))) : v,
  );

/** Whether both desks answer the two requests alike: what's timed is the same work. */
const probe = async (): Promise<string | null> => {
  if (astro) return probeAstro(both);
  for (const scenario of [cheap, heavy]) {
    const op = scenario.ops.graphql!;
    for (let i = 0; i < 10; i++) {
      const [r, e] = await Promise.all(sides.map((side) => op(both[side].base, rng(9000 + i), w)));
      if (!r.ok || !e.ok || canonical(r.body) !== canonical(e.body)) {
        return `${scenario.name}, request ${i}:\n  rust   ${canonical(r).slice(0, 300)}\n  elixir ${canonical(e).slice(0, 300)}`;
      }
    }
  }
  return null;
};

// The run ----------------------------------------------------------------------------

type Step = { rep: number; side: Side; level: number; window: Window; capacity: number };
const steps: Step[] = [];
const recoveries: Array<{ rep: number; side: Side; baselineP99: number | null; seconds: NonNullable<Window["seconds"]>; recoveredAfter: number | null }> = [];

const line = (side: Side, level: string, win: Window) => {
  const c = win.cheap;
  const h = win.heavy;
  console.log(
    `    ${side.padEnd(6)} ${level.padStart(5)}  heavy ${String(win.offered).padStart(6)}/s -> ${String(win.goodput).padStart(6)}/s ok` +
      (h ? `, ${h.failed} errors (${h.timeouts} timed out), ${h.dropped} not sent` : "") +
      `   cheap p50 ${c.p50 ?? "-"}  p99 ${c.p99All ?? "-"}  max ${c.max ?? "-"} ms` +
      (c.failed ? `, ${c.failed} errors` : "") +
      `   server ${win.server.cores ?? "-"} cores, ${win.server.maxRssMiB ?? "-"} MiB` +
      (c.late > c.due * 0.01 ? "   (driver late)" : ""),
  );
};

/** Seconds, from the end of the heavy stream, until the cheap stream's p99 stays near its baseline. */
const recoveredAfter = (baselineP99: number | null, seconds: NonNullable<Window["seconds"]>): number | null => {
  const limit = Math.max((baselineP99 ?? 5) * 2, (baselineP99 ?? 5) + 2);
  let last = -1;
  seconds.forEach((s, i) => {
    if (s.failed > 0 || s.p99 === null || s.p99 > limit) last = i;
  });
  return last === seconds.length - 1 ? null : last + 1;
};

const main = async () => {
  console.log(`Run ${dir}`);
  if (!astro) {
    console.log("Seeding each desk's template from the fixture");
    for (const side of sides) console.log(`  ${side}: seeded in ${await seedTemplate(options.pg, both[side], options.fixture!, log(side))} ms`);
  }
  for (const side of sides) await freshDesk(side);
  const differs = await probe();
  if (differs) throw new Error(`the desks answer differently:\n${differs}`);
  console.log("Both desks answer the heavy request alike");

  for (let rep = 0; rep < options.reps; rep++) {
    console.log(`\nRep ${rep + 1} of ${options.reps}`);
    const capacities = {} as Record<Side, number>;
    console.log("  Capacity for the heavy request alone, closed loop, %d clients", options.calibrateClients);
    for (const side of order(rep)) {
      await freshDesk(side);
      capacities[side] = await capacity(both[side]);
      console.log(`    ${side.padEnd(6)} ${capacities[side]}/s`);
      record({ kind: "capacity", rep, side, perSecond: capacities[side] });
    }
    const slower = Math.min(...sides.map((s) => capacities[s]));
    for (const side of order(rep)) {
      console.log(`  ${side}: the cheap stream at ${options.cheapRate}/s, the heavy one ramped${options.basis === "common" ? ` in multiples of ${slower}/s` : ` in multiples of its own ${capacities[side]}/s`}`);
      await freshDesk(side);
      const desk = both[side];
      // The cheap stream alone first: what to compare it with.
      await run(desk, 0, 3000);
      const baseline = await run(desk, 0);
      line(side, "base", baseline);
      steps.push({ rep, side, level: 0, window: baseline, capacity: capacities[side] });
      record({ kind: "step", rep, side, level: 0, capacity: capacities[side], ...baseline });
      for (const level of options.levels) {
        const rate = Math.round((level * (options.basis === "common" ? slower : capacities[side])) * 10) / 10;
        await run(desk, 0, options.settle - options.warmup > 0 ? options.settle - options.warmup : 500);
        const win = await run(desk, rate);
        line(side, `${level}x`, win);
        steps.push({ rep, side, level, window: win, capacity: capacities[side] });
        record({ kind: "step", rep, side, level, capacity: capacities[side], ...win });
        if ((win.server.maxRssMiB ?? 0) > options.maxRssGiB * 1024) {
          console.log(`    ${side}: past ${options.maxRssGiB} GiB resident, so the ramp stops here`);
          break;
        }
      }
      // The heavy stream has stopped. The cheap one carries on, and is watched.
      const recovery = await run(desk, 0, options.recovery, true);
      const after = recoveredAfter(baseline.cheap.p99All, recovery.seconds!);
      recoveries.push({ rep, side, baselineP99: baseline.cheap.p99All, seconds: recovery.seconds!, recoveredAfter: after });
      record({ kind: "recovery", rep, side, baselineP99: baseline.cheap.p99All, recoveredAfter: after, seconds: recovery.seconds });
      console.log(
        `    ${side.padEnd(6)} recovery: cheap p99 by second ${recovery.seconds!.map((s) => s.p99 ?? "-").join(" ")} ms; ` +
          (after === null ? "not recovered" : `back to baseline after ${after} s`),
      );
    }
  }
  report();
};

// The matrix -------------------------------------------------------------------------

type Arm = { side: Side; pool: number; label: string; env: Record<string, string> };
const equalLimitArms = (): Arm[] =>
  (options.limits ?? []).flatMap((limit) => {
    const pool = options.pools[0];
    return [
      { side: "rust", pool, label: `Rust, ${limit} ms wait limit`, env: { POOL_SIZE: String(pool), POOL_WAIT_MS: String(limit) } },
      {
        side: "elixir",
        pool,
        label: `Elixir, ${limit} ms (queue_target ${limit / 2})`,
        env: { POOL_SIZE: String(pool), POOL_QUEUE_TARGET_MS: String(limit / 2), POOL_QUEUE_INTERVAL_MS: "100" },
      },
    ] as Arm[];
  });
const arms: Arm[] = options.limits ? equalLimitArms() : options.pools.flatMap((pool) => [
  { side: "rust", pool, label: "Rust, queues (no wait limit)", env: { POOL_SIZE: String(pool) } },
  { side: "elixir", pool, label: "Elixir, sheds (Ecto's default)", env: { POOL_SIZE: String(pool) } },
  { side: "rust", pool, label: "Rust, 100 ms wait limit", env: { POOL_SIZE: String(pool), POOL_WAIT_MS: "100" } },
  { side: "elixir", pool, label: "Elixir, queues (queue_target 60 s)", env: { POOL_SIZE: String(pool), POOL_QUEUE_TARGET_MS: "60000", POOL_QUEUE_INTERVAL_MS: "60000" } },
]);
const armResults: Array<{ arm: Arm; rate: number; window: Window }> = [];

const matrix = async () => {
  if (astro) throw new Error("--matrix runs on the supportdesk target");
  console.log(`${options.limits ? "Equal wait limit" : "Pool matrix"}: ${arms.length} configurations, a ${options.window / 1000} s test at each of ${options.rates.join(", ")} heavy requests a second`);
  console.log("Seeding each desk's template from the fixture");
  for (const side of sides) console.log(`  ${side}: seeded in ${await seedTemplate(options.pg, both[side], options.fixture!, log(side))} ms`);
  for (const arm of arms) {
    console.log(`\n${arm.label}, ${arm.pool} connections`);
    for (const key of Object.keys(extraEnv)) delete extraEnv[key];
    Object.assign(extraEnv, arm.env);
    await fresh(options.pg, both[arm.side], log(arm.side));
    const desk = both[arm.side];
    for (const rate of options.rates) {
      const win = await run(desk, rate);
      armResults.push({ arm, rate, window: win });
      record({ kind: "arm", label: arm.label, side: arm.side, pool: arm.pool, env: arm.env, rate, ...win });
      line(arm.side, `${rate}/s`, win);
      console.log(`           postgres: ${win.db?.active ?? "-"} of ${win.db?.connections ?? "-"} connections active, container ${win.db?.containerCpuPct ?? "-"}% CPU`);
      if ((win.server.maxRssMiB ?? 0) > options.maxRssGiB * 1024) {
        console.log(`    past ${options.maxRssGiB} GiB resident, so this configuration stops here`);
        break;
      }
      // What was queued drains, the cheap stream running.
      await run(desk, 0, options.settle);
    }
  }
  matrixReport();
};

const matrixReport = () => {
  const fmt = (v: number | null | undefined, d = 1) => (v === null || v === undefined ? "-" : String(Math.round(v * 10 ** d) / 10 ** d));
  const lines = [
    `# ${options.limits ? "Equal wait limit" : "Pool matrix"}, ${manifest.started}`,
    "",
    `ash-rust at \`${manifest.git.rev.slice(0, 10)}\`${manifest.git.dirty ? " (uncommitted changes)" : ""} against Ash (Elixir) on ${manifest.machine.cpu} (${manifest.machine.cores} cores), ${(manifest as any).postgres.split(" on ")[0]}. Machine load at the start: ${manifest.machine.loadAverageAtStart ?? "unknown"}.`,
    "",
    options.limits
      ? `Each row is one ${options.window / 1000} s test (after ${options.warmup / 1000} s of warm-up) of the same two streams as the ramp: the cheap one, ${options.cheapRate}/s, and the heavy one at the rate named, the same for every configuration, on pools of ${options.pools[0]} connections. Both desks are given the same limit on how long a request waits for a connection: Rust's is \`PoolSettings::wait_timeout\`, which fails a statement that has waited that long. Ecto has no hard limit: once its pool has been slow for an interval it drops what has waited past twice its \`queue_target\`, so Elixir is given a \`queue_target\` of half the limit and a \`queue_interval\` of 100 ms. One rep each: read the differences as indications. Postgres columns are sampled during the window: its connections in use by the desk's database (of those open), and its container's CPU as a share of one core.`
      : `Each row is one ${options.window / 1000} s test (after ${options.warmup / 1000} s of warm-up) of the same two streams as the ramp: the cheap one, ${options.cheapRate}/s, and the heavy one at the rate named, the same for every configuration. Rust's pool either queues without limit (its default) or fails a statement that has waited 100 ms (\`PoolSettings::wait_timeout\`). Elixir's either sheds, as Ecto's \`queue_target\` of 50 ms does (the default), or has its \`queue_target\` and \`queue_interval\` set to 60 s, so it queues. Pool sizes are ${options.pools.join(" and ")}. One rep each: read the differences between configurations as indications, not as measured to the noise. Postgres columns are sampled during the window: its connections in use by the desk's database (of those open), and its container's CPU as a share of one core.`,
    "",
    "| Heavy offered | Configuration | Pool | Cheap p50 / p99 ms | Cheap errors | Heavy answered/s | Heavy p50 / p99 ms | Heavy errors | Heavy not sent | Server cores | Peak RSS MiB | PG active / open | PG container CPU % |",
    "|---|---|---:|---|---:|---:|---|---:|---:|---:|---:|---|---:|",
  ];
  for (const rate of options.rates) {
    for (const { arm, window: win } of armResults.filter((r) => r.rate === rate)) {
      const c = win.cheap;
      const h = win.heavy!;
      lines.push(
        `| ${rate}/s | ${arm.label} | ${arm.pool} | ${fmt(c.p50)} / ${fmt(c.p99All)} | ${c.failed} (${fmt((100 * c.failed) / (c.due || 1), 0)}%) | ${fmt(win.goodput, 0)} | ${fmt(h.p50)} / ${fmt(h.p99All)} | ${h.failed} (${fmt((100 * h.failed) / (h.due || 1), 0)}%) | ${h.dropped} | ${fmt(win.server.cores)} | ${fmt(win.server.maxRssMiB, 0)} | ${fmt(win.db?.active)} / ${fmt(win.db?.connections)} | ${fmt(win.db?.containerCpuPct, 0)} |`,
      );
    }
  }
  writeFileSync(`${dir}/report.md`, lines.join("\n") + "\n");
  console.log(`\n${lines.join("\n")}\n\nWritten to ${dir}`);
};

// The report -------------------------------------------------------------------------

const report = () => {
  const levels = [0, ...options.levels];
  const pick = (side: Side, level: number, f: (w: Window, s: Step) => number | null): number | null => {
    const values = steps
      .filter((s) => s.side === side && s.level === level)
      .map((s) => f(s.window, s))
      .filter((v): v is number => v !== null);
    return values.length ? median(values) : null;
  };
  const fmt = (v: number | null, digits = 1) => (v === null ? "-" : String(Math.round(v * 10 ** digits) / 10 ** digits));
  const caps = (side: Side) => median(steps.filter((s) => s.side === side).map((s) => s.capacity));
  const lines = [
    `# Mixed saturation${astro ? ", CPU-bound" : ""}, ${manifest.started}`,
    "",
    `ash-rust at \`${manifest.git.rev.slice(0, 10)}\`${manifest.git.dirty ? " (uncommitted changes)" : ""} against Ash (Elixir) on ${manifest.machine.cpu} (${manifest.machine.cores} cores), ${astro ? "in memory, no database" : (manifest as any).postgres.split(" on ")[0]}. Machine load at the start: ${manifest.machine.loadAverageAtStart ?? "unknown"}.`,
    "",
    `A cheap stream (${astro ? "`{ __typename }` over GraphQL" : "one ticket by id"}, ${options.cheapRate}/s) and a heavy one (${astro ? `a sort, filter and count over ${options.tickets} tickets in memory, answering a page of 25` : "the 250 newest tickets of an org, with relationships and aggregates"}) arrive together, each at a fixed rate; the heavy one is ramped in ${options.window / 1000} s steps, with ${options.settle / 1000} s of the cheap stream alone between, then stops and the cheap stream is watched. ${options.reps} reps; each figure is the median over reps. Latency is from when a request was due; one unanswered after ${options.timeout / 1000} s is given up on and counted as a failure at that latency. The driver holds at most ${options.maxInFlight} heavy requests unanswered; one due past that is *not sent*, which is the driver's limit and no desk's refusal, and a desk that queues rather than refuses can't queue more than that.`,
    "",
    `Heavy capacity (closed loop, ${options.calibrateClients} clients, the request alone): **ash-rust ${fmt(caps("rust"), 0)}/s, Ash ${fmt(caps("elixir"), 0)}/s**. ` +
      (options.basis === "common"
        ? `Each step offers both desks the same heavy rate, a multiple of the slower desk's capacity.`
        : `Each step offers each desk a multiple of its own capacity.`),
    "",
    "## The cheap stream while the heavy one ramps",
    "",
    "| Heavy offered | " + sides.map((s) => `${s === "rust" ? "Rust" : "Elixir"} cheap p50 / p99 / max ms | ${s === "rust" ? "Rust" : "Elixir"} cheap errors`).join(" | ") + " |",
    "|---|" + "---|---|".repeat(sides.length),
  ];
  for (const level of levels) {
    const offered = (side: Side) => pick(side, level, (win) => win.offered);
    const label = level === 0 ? "none" : options.basis === "common" ? `${level}× (${fmt(offered("rust"), 0)}/s)` : `${level}× own (${fmt(offered("rust"), 0)} / ${fmt(offered("elixir"), 0)}/s)`;
    lines.push(
      `| ${label} | ` +
        sides
          .map((side) => `${fmt(pick(side, level, (win) => win.cheap.p50))} / ${fmt(pick(side, level, (win) => win.cheap.p99All))} / ${fmt(pick(side, level, (win) => win.cheap.max), 0)} | ${fmt(pick(side, level, (win) => win.cheap.failed), 0)}`)
          .join(" | ") +
        " |",
    );
  }
  lines.push(
    "",
    "p99 counts every cheap request due in the window, an error or a timeout at the latency it gave up at. Cheap *failed* are requests the desk answered with an error.",
    "",
    "## The heavy stream: what the desk does with what it's offered",
    "",
    "| Heavy offered | " + sides.map((s) => `${s === "rust" ? "Rust" : "Elixir"} answered/s | ${s === "rust" ? "Rust" : "Elixir"} errors (timed out) | ${s === "rust" ? "Rust" : "Elixir"} not sent | ${s === "rust" ? "Rust" : "Elixir"} p50 ms`).join(" | ") + " |",
    "|---|" + "---|---|---|---|".repeat(sides.length),
  );
  for (const level of options.levels) {
    const offered = (side: Side) => pick(side, level, (win) => win.offered);
    const label = options.basis === "common" ? `${level}× (${fmt(offered("rust"), 0)}/s)` : `${level}× own (${fmt(offered("rust"), 0)} / ${fmt(offered("elixir"), 0)}/s)`;
    lines.push(
      `| ${label} | ` +
        sides
          .map(
            (side) =>
              `${fmt(pick(side, level, (win) => win.goodput), 0)} | ${fmt(pick(side, level, (win) => win.heavy?.failed ?? 0), 0)} (${fmt(pick(side, level, (win) => win.heavy?.timeouts ?? 0), 0)}) | ${fmt(pick(side, level, (win) => win.heavy?.dropped ?? 0), 0)} | ${fmt(pick(side, level, (win) => win.heavy?.p50 ?? null))}`,
          )
          .join(" | ") +
        " |",
    );
  }
  lines.push(
    "",
    "## The server while it's loaded",
    "",
    "| Heavy offered | " + sides.map((s) => `${s === "rust" ? "Rust" : "Elixir"} cores busy | ${s === "rust" ? "Rust" : "Elixir"} peak RSS MiB`).join(" | ") + " |",
    "|---|" + "---|---|".repeat(sides.length),
  );
  for (const level of levels) {
    const offered = (side: Side) => pick(side, level, (win) => win.offered);
    const label = level === 0 ? "none" : options.basis === "common" ? `${level}× (${fmt(offered("rust"), 0)}/s)` : `${level}× own`;
    lines.push(
      `| ${label} | ` +
        sides.map((side) => `${fmt(pick(side, level, (win) => win.server.cores))} | ${fmt(pick(side, level, (win) => win.server.maxRssMiB), 0)}`).join(" | ") +
        " |",
    );
  }
  lines.push("", "## Recovery", "", "After the heavy stream stops, seconds until the cheap stream's p99 stays within 2× its baseline (or +2 ms) with no failures:", "");
  for (const side of sides) {
    const rows = recoveries.filter((r) => r.side === side);
    lines.push(
      `- **${side === "rust" ? "ash-rust" : "Ash"}**: ${rows.map((r) => (r.recoveredAfter === null ? "not within the window" : `${r.recoveredAfter} s`)).join(", ")} (baseline p99 ${rows.map((r) => `${fmt(r.baselineP99)} ms`).join(", ")})`,
    );
  }
  lines.push(
    "",
    astro
      ? "Neither desk has a database, so the heavy request costs the desk CPU and nothing else, and the cheap one costs it almost nothing: the cheap stream's latency is how soon a worker gets to it. The desks and the driver share one machine; the `server` columns say how much of it the desk used."
      : "Postgres, both desks and the driver share one machine, and the heavy request spends much of its time in Postgres, which costs the same on both sides; the figures compare how the desks treat a small request while a large one fills them, not their capacity. Whether the load is ever beyond the driver or the database rather than the desk is for the `server` and `driver late` columns of the run's log to say.",
  );
  writeFileSync(`${dir}/report.md`, lines.join("\n") + "\n");
  // The same run as charts: chart.html, to hover over, and chart.svg, to embed.
  spawnSync(process.execPath, [`${here}chart.ts`, dir], { stdio: "inherit" });
  console.log(`\n${lines.join("\n")}\n\nWritten to ${dir}`);
};

try {
  await (options.matrix ? matrix() : main());
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  for (const side of sides) await stop(both[side]);
  await Promise.all(threads.map((t) => t.terminate()));
}
