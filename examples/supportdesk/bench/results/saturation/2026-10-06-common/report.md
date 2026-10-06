# Mixed saturation, 2026-10-06T00:08:34.639Z

ash-rust at `373729b15b` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), PostgreSQL 16.15 (Debian 16.15-1.pgdg12+2). Machine load at the start: 8.89 15.08 12.80.

A cheap stream (one ticket by id, 500/s) and a heavy one (the 250 newest tickets of an org, with relationships and aggregates) arrive together, each at a fixed rate; the heavy one is ramped in 10 s steps, with 4 s of the cheap stream alone between, then stops and the cheap stream is watched. 2 reps; each figure is the median over reps. Latency is from when a request was due; one unanswered after 10 s is given up on and counted as a failure at that latency. The driver holds at most 3000 heavy requests unanswered; one due past that is *not sent*, which is the driver's limit and no desk's refusal, and a desk that queues rather than refuses can't queue more than that.

Heavy capacity (closed loop, 24 clients, the request alone): **ash-rust 703/s, Ash 265/s**. Each step offers both desks the same heavy rate, a multiple of the slower desk's capacity.

## The cheap stream while the heavy one ramps

| Heavy offered | Rust cheap p50 / p99 / max ms | Rust cheap errors | Elixir cheap p50 / p99 / max ms | Elixir cheap errors |
|---|---|---|---|---|
| none | 1 / 7.2 / 33 | 0 | 1.5 / 4.5 / 24 | 0 |
| 0.25× (66/s) | 1.1 / 8.4 / 21 | 0 | 1.3 / 5 / 14 | 0 |
| 0.5× (133/s) | 1.1 / 9.8 / 43 | 0 | 1.4 / 5.1 / 18 | 0 |
| 1× (265/s) | 0.9 / 8.9 / 46 | 0 | 53.6 / 167.5 / 537 | 63 |
| 1.5× (398/s) | 0.8 / 5.4 / 10 | 0 | 143.5 / 799.8 / 2162 | 910 |
| 2× (531/s) | 0.9 / 16.4 / 45 | 0 | 123 / 2069.4 / 2776 | 1399 |
| 3× (796/s) | 498 / 826.2 / 841 | 0 | 108.1 / 1554.2 / 1600 | 2006 |
| 4× (1062/s) | 1310.3 / 1621.4 / 1636 | 0 | 108.8 / 1763.2 / 1869 | 2447 |

p99 counts every cheap request due in the window, an error or a timeout at the latency it gave up at. Cheap *failed* are requests the desk answered with an error.

## The heavy stream: what the desk does with what it's offered

| Heavy offered | Rust answered/s | Rust errors (timed out) | Rust not sent | Rust p50 ms | Elixir answered/s | Elixir errors (timed out) | Elixir not sent | Elixir p50 ms |
|---|---|---|---|---|---|---|---|---|
| 0.25× (66/s) | 66 | 0 (0) | 0 | 17.8 | 66 | 0 (0) | 0 | 40.1 |
| 0.5× (133/s) | 133 | 0 (0) | 0 | 15.7 | 133 | 0 (0) | 0 | 41.5 |
| 1× (265/s) | 265 | 0 (0) | 0 | 15.7 | 245 | 135 (0) | 0 | 278.8 |
| 1.5× (398/s) | 398 | 0 (0) | 0 | 16.1 | 211 | 2128 (0) | 0 | 488.5 |
| 2× (531/s) | 531 | 0 (0) | 0 | 16.7 | 174 | 3937 (0) | 0 | 467.9 |
| 3× (796/s) | 654 | 0 (0) | 0 | 1593.3 | 113 | 7115 (0) | 0 | 391.7 |
| 4× (1062/s) | 631 | 0 (0) | 2211 | 3957.3 | 75 | 9544 (0) | 571 | 393.6 |

## The server while it's loaded

| Heavy offered | Rust cores busy | Rust peak RSS MiB | Elixir cores busy | Elixir peak RSS MiB |
|---|---|---|---|---|
| none | 0.1 | 15 | 0.5 | 416 |
| 0.25× (66/s) | 0.6 | 34 | 3.1 | 526 |
| 0.5× (133/s) | 1.3 | 62 | 6.1 | 577 |
| 1× (265/s) | 2.9 | 92 | 10.6 | 2286 |
| 1.5× (398/s) | 4.5 | 102 | 10.2 | 4992 |
| 2× (531/s) | 6.4 | 140 | 9.8 | 6295 |
| 3× (796/s) | 8.6 | 3430 | 9.7 | 7997 |
| 4× (1062/s) | 8.4 | 6304 | 9.6 | 9745 |

## Recovery

After the heavy stream stops, seconds until the cheap stream's p99 stays within 2× its baseline (or +2 ms) with no failures:

- **ash-rust**: 0 s, 4 s (baseline p99 9.2 ms, 5.1 ms)
- **Ash**: not within the window, not within the window (baseline p99 4.4 ms, 4.7 ms)

Postgres, both desks and the driver share one machine, and the heavy request spends much of its time in Postgres, which costs the same on both sides; the figures compare how the desks treat a small request while a large one fills them, not their capacity. Whether the load is ever beyond the driver or the database rather than the desk is for the `server` and `driver late` columns of the run's log to say.
