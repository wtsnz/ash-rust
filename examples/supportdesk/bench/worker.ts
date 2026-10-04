// One of the driver's load threads: runs a scenario's requests against one desk, closed
// loop (each of its clients asking again as soon as answered) or open loop (at its share
// of a fixed rate, each request's latency counted from when it was due, so a desk that
// falls behind is charged for the wait).

import { parentPort, workerData } from "node:worker_threads";
import { setTimeout as sleep } from "node:timers/promises";
import { rng, scenarios, type Transport, type World } from "./scenarios.ts";

const w: World = workerData.world;
const index: number = workerData.index;

export type Job = {
  scenario: string;
  transport: Transport;
  base: string;
  seed: number;
  /** When sending starts, and measuring starts and ends (epoch ms): requests due
   *  before `from` warm up. */
  start: number;
  from: number;
  until: number;
  /** Closed loop: this thread's clients. */
  clients?: number;
  /** Open loop: this thread's requests a second, and its offset in the schedule. */
  rate?: number;
  phase?: number;
};

export type JobResult = {
  latencies: Float64Array;
  errors: number;
  /** Every request that succeeded, warming up or measured. */
  succeeded: number;
  /** Requests unanswered when the window closed and the drain ran out. */
  unfinished: number;
  /** Open loop: requests sent more than 20 ms after they were due (a busy driver). */
  late: number;
  firstErrors: unknown[];
};

const run = async (job: Job): Promise<JobResult> => {
  const op = scenarios.find((s) => s.name === job.scenario)!.ops[job.transport]!;
  const latencies: number[] = [];
  let errors = 0;
  let succeeded = 0;
  let late = 0;
  let pending = 0;
  const firstErrors: unknown[] = [];
  const settle = (started: number, measured: boolean, outcome: { ok: boolean; body: unknown } | Error) => {
    const ok = !(outcome instanceof Error) && outcome.ok;
    if (ok) succeeded += 1;
    if (!measured) return;
    if (ok) latencies.push(performance.timeOrigin + performance.now() - started);
    else {
      errors += 1;
      if (firstErrors.length < 3) firstErrors.push(outcome instanceof Error ? String(outcome) : (outcome as any).body);
    }
  };
  const now = () => performance.timeOrigin + performance.now();

  if (job.clients) {
    await Promise.all(
      Array.from({ length: job.clients }, async (_, client) => {
        const r = rng(job.seed ^ (index * 7919 + client * 104729));
        while (now() < job.until) {
          const started = now();
          const measured = started >= job.from;
          try {
            settle(started, measured, await op(job.base, r, w));
          } catch (error) {
            settle(started, measured, error as Error);
          }
        }
      }),
    );
  } else {
    const rate = job.rate!;
    for (let i = 0; ; i++) {
      const due = job.start + ((i + job.phase!) * 1000) / rate;
      if (due >= job.until) break;
      const wait = due - now();
      if (wait > 0) await sleep(wait);
      else if (-wait > 20) late += 1;
      // The same request for the same slot, however the timing falls.
      const r = rng(job.seed ^ (index * 7919 + i * 104729));
      pending += 1;
      op(job.base, r, w)
        .then((outcome) => settle(due, due >= job.from, outcome))
        .catch((error) => settle(due, due >= job.from, error))
        .finally(() => (pending -= 1));
    }
    const drain = now() + 15_000;
    while (pending > 0 && now() < drain) await sleep(5);
  }
  return { latencies: Float64Array.from(latencies), errors, succeeded, unfinished: pending, late, firstErrors };
};

parentPort!.on("message", async (job: Job) => {
  const result = await run(job);
  parentPort!.postMessage(result, [result.latencies.buffer as ArrayBuffer]);
});
