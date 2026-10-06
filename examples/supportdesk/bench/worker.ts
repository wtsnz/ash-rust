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
  /** Keep each request's outcome by the second it was due in (`JobResult.timeline`). */
  timeline?: boolean;
  /** Open loop: this thread's requests a second, and its offset in the schedule. */
  rate?: number;
  phase?: number;
  /** Open loop: give up on a request this long after it's due, as a client would, so a
   *  desk that's fallen behind is charged for the wait without the driver's own backlog
   *  growing without end. */
  timeoutMs?: number;
  /** Open loop: requests this thread has unanswered at most; past it a due request is
   *  counted as dropped, not sent. A guard for the driver, set above what a desk is
   *  asked to hold. */
  maxInFlight?: number;
};

export type JobResult = {
  /** How long each operation started (closed loop) or due (open loop) within the window
   *  took, however late it finished: those that succeeded, and those that failed. */
  latencies: Float64Array;
  failures: Float64Array;
  errors: number;
  /** Operations that succeeded and finished within the window: its throughput. */
  completed: number;
  /** Every request that succeeded, warming up or measured. */
  succeeded: number;
  /** Requests unanswered when the window closed and the drain ran out. */
  unfinished: number;
  /** Open loop: requests sent more than 20 ms after they were due (a busy driver). */
  late: number;
  /** Open loop: requests the driver gave up on at `timeoutMs`, among the failures. */
  timeouts?: number;
  /** Open loop: requests due while `maxInFlight` were unanswered, not sent. */
  dropped?: number;
  /** Open loop: the latency of every request due in the window, by the second it was due
   *  in (from `from`): `[second, latency, ok]` triples, kept when the job asks. */
  timeline?: Array<[number, number, number]>;
  firstErrors: unknown[];
};

const run = async (job: Job): Promise<JobResult> => {
  const op = scenarios.find((s) => s.name === job.scenario)!.ops[job.transport]!;
  const latencies: number[] = [];
  const failures: number[] = [];
  let errors = 0;
  let completed = 0;
  let succeeded = 0;
  let late = 0;
  let timeouts = 0;
  let dropped = 0;
  let pending = 0;
  const timeline: Array<[number, number, number]> = [];
  const firstErrors: unknown[] = [];
  const now = () => performance.timeOrigin + performance.now();
  const settle = (started: number, outcome: { ok: boolean; body: unknown } | Error) => {
    const ok = !(outcome instanceof Error) && outcome.ok;
    const finished = now();
    if (outcome instanceof Error && outcome.name === "TimeoutError" && started >= job.from && started < job.until) timeouts += 1;
    if (job.timeline && started >= job.from && started < job.until) {
      timeline.push([Math.floor((started - job.from) / 1000), finished - started, ok ? 1 : 0]);
    }
    if (ok) succeeded += 1;
    if (ok && finished >= job.from && finished <= job.until) completed += 1;
    if (started < job.from || started >= job.until) return;
    if (ok) latencies.push(finished - started);
    else {
      errors += 1;
      failures.push(finished - started);
      if (firstErrors.length < 3) firstErrors.push(outcome instanceof Error ? String(outcome) : (outcome as any).body);
    }
  };

  if (job.clients) {
    await Promise.all(
      Array.from({ length: job.clients }, async (_, client) => {
        const r = rng(job.seed ^ (index * 7919 + client * 104729));
        while (now() < job.until) {
          const started = now();
          try {
            settle(started, await op(job.base, r, w));
          } catch (error) {
            settle(started, error as Error);
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
      if (job.maxInFlight && pending >= job.maxInFlight) {
        dropped += 1;
        continue;
      }
      pending += 1;
      op(job.base, r, w, job.timeoutMs ? AbortSignal.timeout(job.timeoutMs) : undefined)
        .then((outcome) => settle(due, outcome))
        .catch((error) => settle(due, error))
        .finally(() => (pending -= 1));
    }
    const drain = now() + 15_000;
    while (pending > 0 && now() < drain) await sleep(5);
  }
  return {
    latencies: Float64Array.from(latencies),
    failures: Float64Array.from(failures),
    errors,
    completed,
    succeeded,
    unfinished: pending,
    late,
    timeouts,
    dropped,
    ...(job.timeline ? { timeline } : {}),
    firstErrors,
  };
};

parentPort!.on("message", async (job: Job) => {
  const result = await run(job);
  parentPort!.postMessage(result, [result.latencies.buffer as ArrayBuffer, result.failures.buffer as ArrayBuffer]);
});
