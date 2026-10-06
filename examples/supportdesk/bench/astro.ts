// The in-memory twins the CPU-bound saturation benchmark runs on: the astro-helpdesk server
// in ash-rust and in Ash for Elixir. Neither has a database, so a request that costs a
// desk time costs it CPU, which is what a scheduler hands out.

import { spawn } from "node:child_process";
import { mkdirSync, openSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";

import { listener, stop, type Desk, type Side } from "./desks.ts";

const repo = fileURLToPath(new URL("../../../", import.meta.url));

export const astroBinaries: Record<Side, string> = {
  rust: `${repo}target/release/astro-helpdesk-server`,
  elixir: `${repo}examples/elixir/astro-helpdesk/_build/prod/rel/astro_helpdesk/bin/astro_helpdesk`,
};

export const astroDesks = (): Record<Side, Desk> => {
  const desk = (side: Side, port: number): Desk => ({ side, port, base: `http://127.0.0.1:${port}`, db: "", template: "" });
  return { rust: desk("rust", 4721), elixir: desk("elixir", 4722) };
};

/** Starts `desk` empty of tickets but its three seeded ones, and waits until it serves. */
export const startAstro = async (desk: Desk, log: string) => {
  await stop(desk);
  if (listener(desk.port)) throw new Error(`something already listens on ${desk.port}`);
  // The Rust server writes its TypeScript SDK under its working directory when it starts.
  const cwd = `/tmp/astro-saturation-${desk.side}`;
  mkdirSync(cwd, { recursive: true });
  const out = openSync(log, "a");
  const env = { ...process.env, PORT: String(desk.port), RELEASE_DISTRIBUTION: "none" };
  desk.proc = spawn(astroBinaries[desk.side], desk.side === "elixir" ? ["start"] : [], { env, cwd, stdio: ["ignore", out, out], detached: true });
  const deadline = Date.now() + 120_000;
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

const OPEN = `mutation($input: OpenTicketInput!) { openTicket(input: $input) { result { id } errors { message } } }`;

/** Opens `count` tickets, the same titles on both desks, `parallel` at a time. */
export const seedTickets = async (desk: Desk, count: number, parallel = 32) => {
  let next = 0;
  const worker = async () => {
    for (let i = next++; i < count; i = next++) {
      // Deterministic, scattered titles, so sorting and filtering have something to do.
      const title = `Ticket ${((i * 2654435761) >>> 0).toString(36)} ${(i % 97).toString(16)}`;
      const response = await fetch(`${desk.base}/graphql`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ query: OPEN, variables: { input: { title, priority: 1 + (i % 5), status: "OPEN" } } }),
      });
      const body = await response.json();
      if (!body?.data?.openTicket?.result) throw new Error(`seeding ${desk.side}: ${JSON.stringify(body).slice(0, 200)}`);
    }
  };
  await Promise.all(Array.from({ length: parallel }, worker));
};

const LIST = `query { listTickets(first: 25, sort: [{ field: TITLE, order: DESC }], filter: { title: { contains: "a" } }) { count results { title status priority } } }`;

/** Whether both desks answer the heavy request alike (their ids differ, so not by id). */
export const probeAstro = async (desks: Record<Side, Desk>): Promise<string | null> => {
  const answers = await Promise.all(
    (["rust", "elixir"] as Side[]).map(async (side) => {
      const response = await fetch(`${desks[side].base}/graphql`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ query: LIST }) });
      return (await response.json())?.data?.listTickets;
    }),
  );
  const [r, e] = answers;
  const titles = (page: any) => JSON.stringify(page?.results?.map((t: any) => t.title));
  if (!r || !e || r.count !== e.count || titles(r) !== titles(e)) {
    return `the heavy request:\n  rust   ${JSON.stringify(r).slice(0, 300)}\n  elixir ${JSON.stringify(e).slice(0, 300)}`;
  }
  return null;
};
