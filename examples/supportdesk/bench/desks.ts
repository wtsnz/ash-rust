// Starting, stopping and resetting the two desks. Each desk's data is seeded once a run
// into a template database, then every rep runs on a fresh copy of it (`CREATE DATABASE
// … TEMPLATE`), so each starts from the same rows, statistics and all.

import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { openSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

export type Side = "rust" | "elixir";

const repo = fileURLToPath(new URL("../../../", import.meta.url));
export const binaries: Record<Side, string> = {
  rust: `${repo}target/release/supportdesk`,
  elixir: `${repo}examples/elixir/supportdesk/_build/prod/rel/supportdesk/bin/supportdesk`,
};

export type Desk = {
  side: Side;
  port: number;
  base: string;
  db: string;
  template: string;
  proc?: ChildProcess;
  /** The process serving: the one listening on the port. */
  pid?: number;
};

export const desks = (): Record<Side, Desk> => {
  const desk = (side: Side, port: number): Desk => ({
    side,
    port,
    base: `http://127.0.0.1:${port}`,
    db: `supportdesk_bench_${side}`,
    template: `supportdesk_bench_${side}_tpl`,
  });
  return { rust: desk("rust", 4711), elixir: desk("elixir", 4712) };
};

/** Runs `sql` against `db`, failing loudly. */
export const psql = (pg: string, sql: string, db = "postgres"): string => {
  const out = spawnSync("psql", [`${pg}/${db}`, "-v", "ON_ERROR_STOP=1", "-qAt", "-c", sql], { encoding: "utf8" });
  if (out.status !== 0) throw new Error(`psql: ${sql}\n${out.stderr}`);
  return out.stdout.trim();
};

const recreate = (pg: string, db: string, template?: string) => {
  psql(pg, `DROP DATABASE IF EXISTS "${db}" WITH (FORCE)`);
  psql(pg, template ? `CREATE DATABASE "${db}" TEMPLATE "${template}"` : `CREATE DATABASE "${db}"`);
};

export const listener = (port: number): number | undefined => {
  const out = spawnSync("lsof", ["-ti", `tcp:${port}`, "-sTCP:LISTEN"], { encoding: "utf8" });
  const pid = Number(out.stdout.trim().split("\n")[0]);
  return Number.isFinite(pid) && pid > 0 ? pid : undefined;
};

/** Starts `desk` on `db`, loading `fixture` first if given, and waits until it serves. */
export const start = async (pg: string, desk: Desk, db: string, log: string, fixture?: string) => {
  if (listener(desk.port)) throw new Error(`something already listens on ${desk.port}`);
  const env = {
    ...process.env,
    DATABASE_URL: `${pg}/${db}`,
    PORT: String(desk.port),
    ...(fixture ? { FIXTURE: fixture } : {}),
    // One release node at a time, with no distribution: nothing else to reach it.
    RELEASE_DISTRIBUTION: "none",
  };
  if (!fixture) delete (env as Record<string, string | undefined>).FIXTURE;
  const out = openSync(log, "a");
  const args = desk.side === "elixir" ? ["start"] : [];
  desk.proc = spawn(binaries[desk.side], args, { env, stdio: ["ignore", out, out], detached: true });
  const deadline = Date.now() + 180_000;
  while (Date.now() < deadline) {
    if (desk.proc.exitCode !== null) throw new Error(`${desk.side} exited (${desk.proc.exitCode}); see ${log}`);
    try {
      if ((await fetch(`${desk.base}/health`)).status === 200) {
        desk.pid = listener(desk.port);
        return;
      }
    } catch {}
    await sleep(100);
  }
  throw new Error(`${desk.side} never answered /health; see ${log}`);
};

export const stop = async (desk: Desk) => {
  const proc = desk.proc;
  if (!proc || proc.exitCode !== null) return;
  const exited = new Promise((resolve) => proc.once("exit", resolve));
  // The whole group: a release's script and the node it starts.
  try {
    process.kill(-proc.pid!, "SIGTERM");
  } catch {}
  const killed = await Promise.race([exited.then(() => true), sleep(15_000).then(() => false)]);
  if (!killed) {
    try {
      process.kill(-proc.pid!, "SIGKILL");
    } catch {}
    await exited;
  }
  while (listener(desk.port)) await sleep(50);
  desk.proc = undefined;
  desk.pid = undefined;
};

/** Seeds `desk`'s template from the fixture, through the desk's own `seed` actions,
 *  then analyzes it, so every copy plans queries from the same statistics. */
export const seedTemplate = async (pg: string, desk: Desk, fixture: string, log: string) => {
  recreate(pg, desk.template);
  await start(pg, desk, desk.template, log, fixture);
  const seeded = await (await fetch(`${desk.base}/health/seeded`)).json();
  await stop(desk);
  psql(pg, "VACUUM ANALYZE", desk.template);
  return seeded.seeded_ms as number;
};

/** Starts `desk` on a fresh copy of its template. */
export const fresh = async (pg: string, desk: Desk, log: string) => {
  await stop(desk);
  recreate(pg, desk.db, desk.template);
  await start(pg, desk, desk.db, log);
};

/** The serving process's resident memory (KiB) and CPU time so far (seconds). */
export const usage = (pid: number | undefined): { rss: number; cpu: number } | undefined => {
  if (!pid) return undefined;
  const out = spawnSync("ps", ["-o", "rss=,time=", "-p", String(pid)], { encoding: "utf8" }).stdout.trim();
  const [rss, time] = out.split(/\s+/);
  if (!time) return undefined;
  // [[hh:]mm:]ss.cc
  const cpu = time.split(":").reduce((total, part) => total * 60 + Number(part), 0);
  return { rss: Number(rss), cpu };
};
