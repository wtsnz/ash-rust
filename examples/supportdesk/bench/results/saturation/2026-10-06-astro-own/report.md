# Mixed saturation, CPU-bound, 2026-10-06T01:28:57.599Z

ash-rust at `67ec225b4d` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), in memory, no database. Machine load at the start: 11.75 14.78 11.71.

A cheap stream (`{ __typename }` over GraphQL, 500/s) and a heavy one (a sort, filter and count over 5000 tickets in memory, answering a page of 25) arrive together, each at a fixed rate; the heavy one is ramped in 10 s steps, with 4 s of the cheap stream alone between, then stops and the cheap stream is watched. 2 reps; each figure is the median over reps. Latency is from when a request was due; one unanswered after 10 s is given up on and counted as a failure at that latency. The driver holds at most 500 heavy requests unanswered; one due past that is *not sent*, which is the driver's limit and no desk's refusal, and a desk that queues rather than refuses can't queue more than that.

Heavy capacity (closed loop, 24 clients, the request alone): **ash-rust 2163/s, Ash 459/s**. Each step offers each desk a multiple of its own capacity.

## The cheap stream while the heavy one ramps

| Heavy offered | Rust cheap p50 / p99 / max ms | Rust cheap errors | Elixir cheap p50 / p99 / max ms | Elixir cheap errors |
|---|---|---|---|---|
| none | 0.6 / 2.7 / 7 | 0 | 0.7 / 3.7 / 7 | 0 |
| 0.5× own (1082 / 229/s) | 0.7 / 7.2 / 52 | 0 | 0.9 / 5 / 24 | 0 |
| 0.9× own (1947 / 413/s) | 98.6 / 271.6 / 366 | 0 | 0.7 / 9.9 / 49 | 0 |
| 1× own (2163 / 459/s) | 102.4 / 433.9 / 549 | 0 | 4.1 / 17.5 / 40 | 0 |
| 1.25× own (2704 / 573/s) | 176.2 / 557.6 / 660 | 0 | 7.6 / 37.5 / 73 | 0 |
| 1.5× own (3245 / 688/s) | 194.6 / 461.6 / 559 | 0 | 7.7 / 46.6 / 100 | 0 |
| 2× own (4327 / 917/s) | 181 / 484.2 / 593 | 0 | 8 / 42.1 / 79 | 0 |
| 3× own (6490 / 1376/s) | 190.3 / 681.9 / 758 | 0 | 8 / 36.9 / 80 | 0 |

p99 counts every cheap request due in the window, an error or a timeout at the latency it gave up at. Cheap *failed* are requests the desk answered with an error.

## The heavy stream: what the desk does with what it's offered

| Heavy offered | Rust answered/s | Rust errors (timed out) | Rust not sent | Rust p50 ms | Elixir answered/s | Elixir errors (timed out) | Elixir not sent | Elixir p50 ms |
|---|---|---|---|---|---|---|---|---|
| 0.5× own (1082 / 229/s) | 1082 | 0 (0) | 0 | 4.7 | 229 | 0 (0) | 0 | 13.6 |
| 0.9× own (1947 / 413/s) | 1922 | 0 (0) | 158 | 100.9 | 403 | 0 (0) | 0 | 15.1 |
| 1× own (2163 / 459/s) | 2038 | 0 (0) | 1274 | 105.1 | 419 | 0 (0) | 286 | 618.4 |
| 1.25× own (2704 / 573/s) | 2176 | 0 (0) | 5904 | 185.5 | 396 | 0 (0) | 1972 | 1241.9 |
| 1.5× own (3245 / 688/s) | 2183 | 0 (0) | 12270 | 201.9 | 363 | 0 (0) | 3656 | 1326.7 |
| 2× own (4327 / 917/s) | 2165 | 0 (0) | 25516 | 205.4 | 368 | 0 (0) | 6309 | 1333.8 |
| 3× own (6490 / 1376/s) | 2149 | 0 (0) | 51568 | 197.1 | 376 | 0 (0) | 11748 | 1296.7 |

## The server while it's loaded

| Heavy offered | Rust cores busy | Rust peak RSS MiB | Elixir cores busy | Elixir peak RSS MiB |
|---|---|---|---|---|
| none | 0 | 18 | 0.2 | 363 |
| 0.5× own | 5 | 98 | 5.9 | 632 |
| 0.9× own | 11.1 | 128 | 11.8 | 1485 |
| 1× own | 11.8 | 133 | 13.5 | 3569 |
| 1.25× own | 13.1 | 141 | 13.7 | 6152 |
| 1.5× own | 13.3 | 144 | 13.1 | 6227 |
| 2× own | 12.9 | 144 | 13.1 | 6310 |
| 3× own | 12.9 | 144 | 13.5 | 6428 |

## Recovery

After the heavy stream stops, seconds until the cheap stream's p99 stays within 2× its baseline (or +2 ms) with no failures:

- **ash-rust**: 0 s, 4 s (baseline p99 2.7 ms, 2.7 ms)
- **Ash**: 14 s, 0 s (baseline p99 3.5 ms, 3.8 ms)

Neither desk has a database, so the heavy request costs the desk CPU and nothing else, and the cheap one costs it almost nothing: the cheap stream's latency is how soon a worker gets to it. The desks and the driver share one machine; the `server` columns say how much of it the desk used.
