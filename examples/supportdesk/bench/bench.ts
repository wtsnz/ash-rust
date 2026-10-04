// Benchmarks the twin desks, ash-rust and Ash in Elixir, through the API real clients
// use: AshTypescript RPC through the generated client, and GraphQL. See README.md.
//
//   node examples/supportdesk/bench/bench.ts --fixture /tmp/fixture.json
//
// Nothing is timed unless both desks answer alike: parity and the smoke test run first,
// and each read scenario's requests are compared before it's timed.

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { cpus, totalmem } from "node:os";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { Worker } from "node:worker_threads";

import { binaries, desks, fresh, psql, seedTemplate, stop, usage, type Desk, type Side } from "./desks.ts";
import { graphql, hot, rng, scenarios, world, type Scenario, type Transport, type Who } from "./scenarios.ts";
import { compare, median, summarize, type Summary } from "./stats.ts";
import { connect } from "./ws.ts";
import type { Job, JobResult } from "./worker.ts";

// Options ----------------------------------------------------------------------------

const args = process.argv.slice(2);
const flag = (name: string, fallback?: string): string | undefined => {
  const at = args.indexOf(name);
  return at >= 0 ? args[at + 1] : fallback;
};
const has = (name: string) => args.includes(name);
const quick = has("--quick");
const options = {
  fixture: flag("--fixture"),
  pg: flag("--pg", "postgres://postgres:postgres@127.0.0.1:55434")!,
  threads: Number(flag("--threads", "6")),
  clients: Number(flag("--clients", "16")),
  warmup: Number(flag("--warmup", quick ? "500" : "1000")),
  window: Number(flag("--window", quick ? "2000" : "5000")),
  readReps: Number(flag("--read-reps", quick ? "1" : "3")),
  writeReps: Number(flag("--write-reps", quick ? "1" : "2")),
  only: flag("--only")?.split(","),
  gate: !has("--skip-gate"),
};
if (!options.fixture) {
  console.error("usage: node bench.ts --fixture fixture.json [--quick] [--only inbox,events] [--threads 4] [--clients 16]");
  process.exit(2);
}
const wanted = (name: string) => !options.only || options.only.includes(name);

// The run's directory and manifest ---------------------------------------------------

const here = fileURLToPath(new URL(".", import.meta.url));
const repo = fileURLToPath(new URL("../../../", import.meta.url));
const stamp = new Date().toISOString().replace(/[:.]/g, "-").slice(0, 19);
const dir = `${here}runs/${stamp}`;
mkdirSync(dir, { recursive: true });
const sh = (cmd: string, cwd = repo) => spawnSync("bash", ["-c", cmd], { cwd, encoding: "utf8" }).stdout.trim();
const hash = (path: string) => createHash("sha256").update(readFileSync(path)).digest("hex").slice(0, 16);

const manifest = {
  started: new Date().toISOString(),
  git: { rev: sh("git rev-parse HEAD"), dirty: sh("git status --porcelain").length > 0 },
  binaries: {
    rust: { path: binaries.rust, sha256: hash(binaries.rust) },
    elixir: { release: binaries.elixir, version: sh("cat examples/elixir/supportdesk/_build/prod/rel/supportdesk/releases/start_erl.data") },
  },
  machine: { cpu: cpus()[0].model, cores: cpus().length, memoryGiB: Math.round(totalmem() / 2 ** 30), os: sh("uname -sr") },
  postgres: psql(options.pg, "SELECT version()"),
  node: process.version,
  fixture: { path: options.fixture, sha256: hash(options.fixture) },
  options,
};
writeFileSync(`${dir}/manifest.json`, JSON.stringify(manifest, null, 2));
const record = (entry: object) => appendFileSync(`${dir}/results.jsonl`, JSON.stringify(entry) + "\n");
const log = (side: Side) => `${dir}/${side}.log`;

const fixture = JSON.parse(readFileSync(options.fixture, "utf8"));
const w = world(fixture);
const both = desks();
const sides: Side[] = ["rust", "elixir"];
/** Which desk goes first in rep `rep`: alternately, so neither always runs warm or cold. */
const order = (rep: number): Side[] => (rep % 2 === 0 ? sides : [...sides].reverse());
const deadline = Date.now() + 15 * 60_000;

// Load threads -----------------------------------------------------------------------

const threads = Array.from(
  { length: options.threads },
  (_, index) => new Worker(new URL("./worker.ts", import.meta.url), { workerData: { world: w, index } }),
);
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

type Window = {
  summary: Summary;
  succeeded: number;
  unfinished: number;
  late: number;
  firstErrors: unknown[];
  server: { cpuSeconds: number | null; cpuPerKRequests: number | null; maxRssMiB: number | null };
  driverCores: number;
};

/** One measured window of `scenario` over `transport` against `desk`. */
const measure = async (scenario: Scenario, transport: Transport, desk: Desk, seed: number): Promise<Window> => {
  const start = Date.now() + 50;
  const from = start + options.warmup;
  const until = from + options.window;
  const n = threads.length;
  const jobs: Job[] = threads.map((_, i) => ({
    scenario: scenario.name,
    transport,
    base: desk.base,
    seed,
    start,
    from,
    until,
    ...(scenario.tier === "read"
      ? { clients: Math.floor(options.clients / n) + (i < options.clients % n ? 1 : 0) }
      : { rate: scenario.rate! / n, phase: i / n }),
  }));
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
  const driverBefore = process.cpuUsage();
  const results = await Promise.all(jobs.map((job, i) => (job.clients === 0 ? emptyResult() : runJob(threads[i], job))));
  const driver = process.cpuUsage(driverBefore);
  await sampling;
  const latencies = Float64Array.from(results.flatMap((r) => Array.from(r.latencies)));
  const errors = results.reduce((t, r) => t + r.errors, 0);
  const summary = summarize(latencies, errors, options.window / 1000);
  const cpuSeconds = before && after ? Math.round((after.cpu - before.cpu) * 100) / 100 : null;
  return {
    summary,
    succeeded: results.reduce((t, r) => t + r.succeeded, 0),
    unfinished: results.reduce((t, r) => t + r.unfinished, 0),
    late: results.reduce((t, r) => t + r.late, 0),
    firstErrors: results.flatMap((r) => r.firstErrors).slice(0, 3),
    server: {
      cpuSeconds,
      cpuPerKRequests: cpuSeconds !== null && summary.count ? Math.round((cpuSeconds / summary.count) * 1000 * 1000) / 1000 : null,
      maxRssMiB: maxRss ? Math.round(maxRss / 1024) : null,
    },
    driverCores: Math.round(((driver.user + driver.system) / 1e6 / ((until - start) / 1000)) * 100) / 100,
  };
};
const emptyResult = async (): Promise<JobResult> => ({
  latencies: new Float64Array(),
  errors: 0,
  succeeded: 0,
  unfinished: 0,
  late: 0,
  firstErrors: [],
});

// Comparing answers ------------------------------------------------------------------

/** JSON with keys in order: the desks needn't encode an object's keys alike. */
const canonical = (value: unknown): string =>
  JSON.stringify(value, (_, v) =>
    v && typeof v === "object" && !Array.isArray(v) ? Object.fromEntries(Object.entries(v).sort(([a], [b]) => a.localeCompare(b))) : v,
  );

/** Whether both desks answer `scenario`'s first requests alike: what's timed is the
 *  same work. Reads only: the desks' data must be untouched. */
const probe = async (scenario: Scenario, transport: Transport): Promise<string | null> => {
  const op = scenario.ops[transport]!;
  for (let i = 0; i < 12; i++) {
    const [r, e] = await Promise.all(sides.map((side) => op(both[side].base, rng(9000 + i), w)));
    if (!r.ok || !e.ok || canonical(r.body) !== canonical(e.body)) {
      return `request ${i}:\n      rust   ${canonical(r).slice(0, 400)}\n      elixir ${canonical(e).slice(0, 400)}`;
    }
  }
  return null;
};

// The gate ---------------------------------------------------------------------------

const gate = async () => {
  console.log("\nGate: parity and the smoke test, on fresh copies");
  for (const side of sides) await fresh(options.pg, both[side], log(side));
  const urls = ["--rust", both.rust.base, "--elixir", both.elixir.base, "--fixture", options.fixture!];
  const parity = spawnSync(`${repo}target/release/parity`, urls, { encoding: "utf8" });
  writeFileSync(`${dir}/parity.txt`, parity.stdout + parity.stderr);
  console.log(`  parity: ${parity.stdout.trim().split("\n").pop()}`);
  // Parity writes; the smoke test reads what it expects of untouched desks.
  for (const side of sides) await fresh(options.pg, both[side], log(side));
  const smoke = spawnSync(process.execPath, [`${here}../client/smoke.ts`, ...urls], { encoding: "utf8" });
  writeFileSync(`${dir}/smoke.txt`, smoke.stdout + smoke.stderr);
  console.log(`  smoke:  ${smoke.stdout.trim().split("\n").pop()}`);
  if (parity.status !== 0 || smoke.status !== 0) throw new Error(`the desks answer differently; see ${dir}/parity.txt and smoke.txt`);
};

// Tiers ------------------------------------------------------------------------------

type Cell = { scenario: string; transport: Transport; tier: string };
const cells = (tier: "read" | "write"): Array<[Scenario, Transport]> =>
  scenarios
    .filter((s) => s.tier === tier && wanted(s.name))
    .flatMap((s) => (Object.keys(s.ops) as Transport[]).map((t) => [s, t] as [Scenario, Transport]));

const results: Array<Cell & { side: Side; rep: number; window: Window; check?: string }> = [];
const invalid: Array<Cell & { why: string }> = [];

const show = (side: Side, rep: number, win: Window, check?: string) => {
  const s = win.summary;
  const notes = [
    s.errors ? `${s.errors} errors` : "",
    win.unfinished ? `${win.unfinished} unfinished` : "",
    win.late ? `${win.late} late` : "",
    check ?? "",
  ].filter(Boolean);
  console.log(
    `    ${side.padEnd(6)} rep ${rep}  ${String(s.rate).padStart(8)}/s  p50 ${s.p50 ?? "-"}  p90 ${s.p90 ?? "-"}  p99 ${s.p99 ?? "-"} ms` +
      `  cpu ${win.server.cpuSeconds ?? "-"}s  rss ${win.server.maxRssMiB ?? "-"}MiB  driver ${win.driverCores} cores` +
      (notes.length ? `  (${notes.join(", ")})` : ""),
  );
  if (s.errors && win.firstErrors.length) console.log(`      first errors: ${JSON.stringify(win.firstErrors).slice(0, 300)}`);
};

const reads = async () => {
  const todo = cells("read");
  if (!todo.length) return;
  console.log("\nReads: closed loop, %d clients, on fresh copies", options.clients);
  for (const side of sides) await fresh(options.pg, both[side], log(side));
  for (const [scenario, transport] of todo) {
    console.log(`  ${scenario.name} over ${transport}`);
    const differs = await probe(scenario, transport);
    if (differs) {
      console.log(`    SKIPPED: the desks answer differently, ${differs}`);
      invalid.push({ scenario: scenario.name, transport, tier: "read", why: differs });
      continue;
    }
    for (let rep = 0; rep < options.readReps; rep++) {
      for (const side of order(rep)) {
        const win = await measure(scenario, transport, both[side], 1000 + rep);
        show(side, rep, win);
        results.push({ scenario: scenario.name, transport, tier: "read", side, rep, window: win });
        record({ tier: "read", scenario: scenario.name, transport, side, rep, ...win });
      }
    }
  }
};

/** The ten hot tickets' view counts, as the first org's admin reads them. */
const viewCounts = async (desk: Desk): Promise<number> => {
  const org = w.orgs[0];
  const admin: Who = { org: org.slug, role: "admin", id: org.admin };
  let total = 0;
  for (const id of hot(w)) {
    const body = await graphql(desk.base, admin, "query T($id: ID!) { getTicket(id: $id) { viewCount } }", { id });
    total += body.data.getTicket.viewCount;
  }
  return total;
};

const writes = async () => {
  const todo = cells("write");
  const events = wanted("events");
  if (!todo.length && !events) return;
  console.log("\nWrites: open loop at fixed rates, each desk on a fresh copy each rep");
  for (let rep = 0; rep < options.writeReps; rep++) {
    for (const side of order(rep)) {
      const desk = both[side];
      await fresh(options.pg, desk, log(side));
      console.log(`  ${side}, rep ${rep}`);
      for (const [scenario, transport] of todo) {
        const viewsBefore = scenario.name === "counters" ? await viewCounts(desk) : 0;
        const win = await measure(scenario, transport, desk, 2000 + rep);
        let check: string | undefined;
        if (scenario.name === "counters") {
          const counted = (await viewCounts(desk)) - viewsBefore;
          check = counted === win.succeeded ? "every acknowledged view counted" : `LOST VIEWS: ${win.succeeded} acknowledged, ${counted} counted`;
        }
        process.stdout.write(`  ${`${scenario.name} over ${transport}`.padEnd(26)}`);
        show(side, rep, win, check);
        results.push({ scenario: scenario.name, transport, tier: "write", side, rep, window: win, check });
        record({ tier: "write", scenario: scenario.name, transport, side, rep, check, ...win });
      }
      if (events) {
        const win = await subscriptions(desk);
        process.stdout.write(`  ${"events over graphql".padEnd(26)}`);
        show(side, rep, win.window, win.check);
        results.push({ scenario: "events", transport: "graphql", tier: "write", side, rep, window: win.window, check: win.check });
        record({ tier: "write", scenario: "events", transport: "graphql", side, rep, check: win.check, ...win.window });
      }
      if (Date.now() > deadline) throw new Error("over the time limit");
    }
  }
};

/**
 * `ticketUpdated` to 20 subscribers of the first org while its hot tickets are viewed
 * 20 times a second over GraphQL: how long each update takes to reach each subscriber,
 * from when the view was sent, and whether every one arrives.
 */
const subscribers = 20;
const eventRate = 20;
const subscriptions = async (desk: Desk) => {
  const org = w.orgs[0];
  const staff = [org.admin, ...org.agents];
  const sentAt = new Map<string, number>();
  const heardAt = new Map<string, number[]>();
  let measuring = false;
  const sockets = await Promise.all(
    Array.from({ length: subscribers }, async (_, i) => {
      const id = staff[i % staff.length];
      const role = id === org.admin ? "admin" : "agent";
      let acked: () => void;
      const ack = new Promise<void>((resolve) => (acked = resolve));
      const socket = await connect(
        desk.base.replace("http", "ws") + "/graphql/ws",
        { "x-org": org.slug, "x-actor": id, "x-role": role },
        "graphql-transport-ws",
        (text) => {
          const message = JSON.parse(text);
          if (message.type === "connection_ack") acked();
          if (message.type === "next" && measuring) {
            const updated = message.payload?.data?.ticketUpdated?.updated;
            if (updated) {
              const key = `${updated.id}:${updated.viewCount}`;
              heardAt.set(key, [...(heardAt.get(key) ?? []), performance.timeOrigin + performance.now()]);
            }
          }
        },
      );
      socket.send(JSON.stringify({ type: "connection_init", payload: {} }));
      await ack;
      socket.send(JSON.stringify({ id: "1", type: "subscribe", payload: { query: "subscription { ticketUpdated { updated { id viewCount } } }" } }));
      return socket;
    }),
  );
  await sleep(500);
  const viewer: Who = { org: org.slug, role: "agent", id: org.agents[0] };
  const r = rng(77);
  const pending: Array<Promise<void>> = [];
  let acknowledged = 0;
  let errors = 0;
  measuring = true;
  const start = performance.timeOrigin + performance.now();
  const total = (eventRate * options.window) / 1000;
  for (let i = 0; i < total; i++) {
    const due = start + (i * 1000) / eventRate;
    const wait = due - (performance.timeOrigin + performance.now());
    if (wait > 0) await sleep(wait);
    const id = hot(w)[Math.floor(r.next() * 10)];
    pending.push(
      graphql(desk.base, viewer, "mutation V($id: ID!) { viewTicket(id: $id) { result { id viewCount } } }", { id })
        .then((body) => {
          const result = body?.data?.viewTicket?.result;
          if (!result) return void (errors += 1);
          acknowledged += 1;
          sentAt.set(`${result.id}:${result.viewCount}`, due);
        })
        .catch(() => void (errors += 1)),
    );
  }
  await Promise.all(pending);
  await sleep(2000);
  measuring = false;
  for (const socket of sockets) socket.close();
  const latencies: number[] = [];
  let heard = 0;
  for (const [key, sent] of sentAt) {
    for (const at of heardAt.get(key) ?? []) {
      latencies.push(at - sent);
      heard += 1;
    }
  }
  const expected = acknowledged * subscribers;
  const summary = summarize(Float64Array.from(latencies), errors, options.window / 1000);
  return {
    window: {
      summary,
      succeeded: acknowledged,
      unfinished: 0,
      late: 0,
      firstErrors: [],
      server: { cpuSeconds: null, cpuPerKRequests: null, maxRssMiB: null },
      driverCores: 0,
    } as Window,
    check: heard === expected ? "every update delivered to every subscriber" : `DELIVERED ${heard} of ${expected}`,
  };
};

// The report -------------------------------------------------------------------------

const report = () => {
  const lines = [
    `# Supportdesk benchmark, ${manifest.started}`,
    "",
    `ash-rust at \`${manifest.git.rev.slice(0, 10)}\`${manifest.git.dirty ? " (uncommitted changes)" : ""} against Ash (Elixir), on ${manifest.machine.cpu} (${manifest.machine.cores} cores), ${manifest.postgres.split(" on ")[0]}.`,
    `Reads: closed loop, ${options.clients} clients, ${options.readReps} reps of ${options.window / 1000} s per desk. Writes: open loop at fixed rates, ${options.writeReps} reps per desk, each on a fresh copy of the seeded data.`,
    "",
    "Each figure is the median over reps. **Ratio** is how many times better ash-rust does (throughput for reads, p50 latency for writes): above 1, ash-rust is ahead. *Within noise* means within 5% or the spread between reps.",
    "",
    "| Scenario | Over | Rust | Elixir | Ratio | Rust p50 / p99 ms | Elixir p50 / p99 ms | Server CPU s per 1k requests (Rust / Elixir) | Checks |",
    "|---|---|---:|---:|---:|---|---|---|---|",
  ];
  const keys = [...new Set(results.map((r) => `${r.tier}|${r.scenario}|${r.transport}`))];
  for (const key of keys) {
    const [tier, scenario, transport] = key.split("|");
    const of = (side: Side) => results.filter((r) => `${r.tier}|${r.scenario}|${r.transport}` === key && r.side === side);
    const [rs, es] = [of("rust"), of("elixir")];
    if (!rs.length || !es.length) continue;
    const pick = (rows: typeof rs, f: (w: Window) => number | null) => {
      const values = rows.map((r) => f(r.window)).filter((v): v is number => v !== null);
      return values.length ? median(values) : null;
    };
    const read = tier === "read";
    const c = read
      ? compare(rs.map((r) => r.window.summary.rate), es.map((r) => r.window.summary.rate), true)
      : compare(rs.map((r) => r.window.summary.p50 ?? 0), es.map((r) => r.window.summary.p50 ?? 0), false);
    const fmt = (v: number | null) => (v === null ? "-" : String(Math.round(v * 100) / 100));
    const headline = (v: number) => (read ? `${Math.round(v)}/s` : `${fmt(v)} ms`);
    const checks = [...new Set([...rs, ...es].map((r) => r.check).filter(Boolean))];
    const errors = [...rs, ...es].reduce((t, r) => t + r.window.summary.errors, 0);
    lines.push(
      `| ${scenario} | ${transport} | ${headline(c.rust)} | ${headline(c.elixir)} | ${c.ratio.toFixed(2)}×${c.withinNoise ? " *within noise*" : ""} | ` +
        `${fmt(pick(rs, (w) => w.summary.p50))} / ${fmt(pick(rs, (w) => w.summary.p99))} | ${fmt(pick(es, (w) => w.summary.p50))} / ${fmt(pick(es, (w) => w.summary.p99))} | ` +
        `${fmt(pick(rs, (w) => w.server.cpuPerKRequests))} / ${fmt(pick(es, (w) => w.server.cpuPerKRequests))} | ` +
        `${[...checks, errors ? `${errors} errors` : ""].filter(Boolean).join("; ") || "ok"} |`,
    );
  }
  if (invalid.length) {
    lines.push("", "## Not timed: the desks answered differently", "");
    for (const cell of invalid) lines.push(`- ${cell.scenario} over ${cell.transport}: ${cell.why.split("\n")[0]}`);
  }
  lines.push(
    "",
    "## Scenarios",
    "",
    ...scenarios.filter((s) => wanted(s.name)).map((s) => `- **${s.name}**${s.rate ? ` (${s.rate}/s)` : ""}: ${s.what}`),
    `- **events** (${eventRate}/s, ${subscribers} subscribers): \`ticketUpdated\` delivered to every subscriber of the org while its hot tickets are viewed over GraphQL; latency is from each view's send to each delivery`,
    "",
    "Postgres, both desks and the driver share one machine. The figures compare the desks with each other; they're not capacity numbers.",
  );
  writeFileSync(`${dir}/report.md`, lines.join("\n") + "\n");
  console.log(`\n${lines.join("\n")}\n\nWritten to ${dir}`);
};

// The run ----------------------------------------------------------------------------

const main = async () => {
  console.log(`Run ${dir}`);
  console.log("Seeding each desk's template from the fixture");
  for (const side of sides) {
    const ms = await seedTemplate(options.pg, both[side], options.fixture!, log(side));
    console.log(`  ${side}: seeded in ${ms} ms`);
  }
  if (options.gate) await gate();
  await reads();
  await writes();
  report();
};

try {
  await main();
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  for (const side of sides) await stop(both[side]);
  await Promise.all(threads.map((t) => t.terminate()));
}
