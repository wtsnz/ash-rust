# Mixed saturation, CPU-bound, 2026-10-06T01:19:09.134Z

ash-rust at `67ec225b4d` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), in memory, no database. Machine load at the start: 7.24 6.51 6.12.

A cheap stream (`{ __typename }` over GraphQL, 500/s) and a heavy one (a sort, filter and count over 5000 tickets in memory, answering a page of 25) arrive together, each at a fixed rate; the heavy one is ramped in 10 s steps, with 4 s of the cheap stream alone between, then stops and the cheap stream is watched. 2 reps; each figure is the median over reps. Latency is from when a request was due; one unanswered after 10 s is given up on and counted as a failure at that latency. The driver holds at most 500 heavy requests unanswered; one due past that is *not sent*, which is the driver's limit and no desk's refusal, and a desk that queues rather than refuses can't queue more than that.

Heavy capacity (closed loop, 24 clients, the request alone): **ash-rust 2141/s, Ash 456/s**. Each step offers both desks the same heavy rate, a multiple of the slower desk's capacity.

## The cheap stream while the heavy one ramps

| Heavy offered | Rust cheap p50 / p99 / max ms | Rust cheap errors | Elixir cheap p50 / p99 / max ms | Elixir cheap errors |
|---|---|---|---|---|
| none | 0.9 / 3.7 / 12 | 0 | 0.8 / 3.8 / 7 | 0 |
| 0.25× (114/s) | 0.9 / 4.8 / 6 | 0 | 0.9 / 2.3 / 19 | 0 |
| 0.5× (228/s) | 1 / 4.7 / 27 | 0 | 0.7 / 2.5 / 33 | 0 |
| 1× (456/s) | 0.8 / 3.8 / 5 | 0 | 5.9 / 32.3 / 81 | 0 |
| 2× (911/s) | 0.9 / 3.3 / 42 | 0 | 8 / 39 / 84 | 0 |
| 4× (1822/s) | 1.2 / 56.2 / 111 | 0 | 8.1 / 40.6 / 74 | 0 |
| 8× (3644/s) | 201.1 / 604.9 / 682 | 0 | 8.1 / 40.2 / 99 | 0 |

p99 counts every cheap request due in the window, an error or a timeout at the latency it gave up at. Cheap *failed* are requests the desk answered with an error.

## The heavy stream: what the desk does with what it's offered

| Heavy offered | Rust answered/s | Rust errors (timed out) | Rust not sent | Rust p50 ms | Elixir answered/s | Elixir errors (timed out) | Elixir not sent | Elixir p50 ms |
|---|---|---|---|---|---|---|---|---|
| 0.25× (114/s) | 114 | 0 (0) | 0 | 5.5 | 114 | 0 (0) | 0 | 14.6 |
| 0.5× (228/s) | 228 | 0 (0) | 0 | 5 | 228 | 0 (0) | 0 | 13.8 |
| 1× (456/s) | 456 | 0 (0) | 0 | 4.7 | 391 | 0 (0) | 448 | 903.3 |
| 2× (911/s) | 911 | 0 (0) | 0 | 4.3 | 379 | 0 (0) | 6106 | 1261 |
| 4× (1822/s) | 1820 | 0 (0) | 0 | 6.2 | 387 | 0 (0) | 16995 | 1259.9 |
| 8× (3644/s) | 2006 | 0 (0) | 19269 | 215.5 | 385 | 0 (0) | 38882 | 1279.9 |

## The server while it's loaded

| Heavy offered | Rust cores busy | Rust peak RSS MiB | Elixir cores busy | Elixir peak RSS MiB |
|---|---|---|---|---|
| none | 0.1 | 18 | 0.2 | 374 |
| 0.25× (114/s) | 0.5 | 25 | 2.9 | 502 |
| 0.5× (228/s) | 0.9 | 81 | 5.8 | 547 |
| 1× (456/s) | 1.9 | 84 | 13.6 | 6045 |
| 2× (911/s) | 3.8 | 91 | 13.5 | 6433 |
| 4× (1822/s) | 10.5 | 111 | 13.7 | 6668 |
| 8× (3644/s) | 13.3 | 135 | 13.5 | 6912 |

## Recovery

After the heavy stream stops, seconds until the cheap stream's p99 stays within 2× its baseline (or +2 ms) with no failures:

- **ash-rust**: 0 s, 0 s (baseline p99 3.4 ms, 3.9 ms)
- **Ash**: 2 s, 11 s (baseline p99 3.7 ms, 3.9 ms)

Neither desk has a database, so the heavy request costs the desk CPU and nothing else, and the cheap one costs it almost nothing: the cheap stream's latency is how soon a worker gets to it. The desks and the driver share one machine; the `server` columns say how much of it the desk used.
