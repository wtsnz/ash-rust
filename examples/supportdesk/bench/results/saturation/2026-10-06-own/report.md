# Mixed saturation, 2026-10-06T00:19:50.673Z

ash-rust at `373729b15b` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), PostgreSQL 16.15 (Debian 16.15-1.pgdg12+2). Machine load at the start: 11.21 17.04 15.75.

A cheap stream (one ticket by id, 500/s) and a heavy one (the 250 newest tickets of an org, with relationships and aggregates) arrive together, each at a fixed rate; the heavy one is ramped in 10 s steps, with 4 s of the cheap stream alone between, then stops and the cheap stream is watched. 2 reps; each figure is the median over reps. Latency is from when a request was due; one unanswered after 10 s is given up on and counted as a failure at that latency. The driver holds at most 3000 heavy requests unanswered; one due past that is *not sent*, which is the driver's limit and no desk's refusal, and a desk that queues rather than refuses can't queue more than that.

Heavy capacity (closed loop, 24 clients, the request alone): **ash-rust 697/s, Ash 266/s**. Each step offers each desk a multiple of its own capacity.

## The cheap stream while the heavy one ramps

| Heavy offered | Rust cheap p50 / p99 / max ms | Rust cheap errors | Elixir cheap p50 / p99 / max ms | Elixir cheap errors |
|---|---|---|---|---|
| none | 1 / 4 / 14 | 0 | 1.5 / 6.5 / 23 | 0 |
| 0.5× own (349 / 133/s) | 0.8 / 6.5 / 36 | 0 | 1.4 / 5 / 36 | 0 |
| 0.9× own (628 / 239/s) | 1.1 / 18.5 / 44 | 0 | 2.4 / 64.2 / 96 | 0 |
| 1× own (697 / 266/s) | 105.7 / 169 / 180 | 0 | 54.5 / 115.2 / 267 | 64 |
| 1.25× own (872 / 333/s) | 756.5 / 1274.2 / 1311 | 0 | 126.1 / 409.1 / 1057 | 540 |
| 1.5× own (1046 / 399/s) | 1308.2 / 1668.7 / 1692 | 0 | 112.4 / 1000.8 / 1373 | 906 |
| 2× own (1394 / 532/s) | 1461.6 / 1659.9 / 1673 | 0 | 109.3 / 1269.9 / 1613 | 1318 |
| 3× own (2092 / 798/s) | 1433.8 / 1693 / 1711 | 0 | 107.4 / 1609.7 / 2023 | 1932 |

p99 counts every cheap request due in the window, an error or a timeout at the latency it gave up at. Cheap *failed* are requests the desk answered with an error.

## The heavy stream: what the desk does with what it's offered

| Heavy offered | Rust answered/s | Rust errors (timed out) | Rust not sent | Rust p50 ms | Elixir answered/s | Elixir errors (timed out) | Elixir not sent | Elixir p50 ms |
|---|---|---|---|---|---|---|---|---|
| 0.5× own (349 / 133/s) | 349 | 0 (0) | 0 | 16.2 | 133 | 0 (0) | 0 | 41.1 |
| 0.9× own (628 / 239/s) | 628 | 0 (0) | 0 | 17.7 | 236 | 0 (0) | 0 | 53.9 |
| 1× own (697 / 266/s) | 678 | 0 (0) | 0 | 336.9 | 252 | 117 (0) | 0 | 266.6 |
| 1.25× own (872 / 333/s) | 645 | 0 (0) | 0 | 2492.9 | 227 | 1156 (0) | 0 | 496 |
| 1.5× own (1046 / 399/s) | 639 | 0 (0) | 2040 | 3914 | 207 | 2158 (0) | 0 | 439.2 |
| 2× own (1394 / 532/s) | 654 | 0 (0) | 6067 | 4247.1 | 169 | 3947 (0) | 0 | 412.4 |
| 3× own (2092 / 798/s) | 675 | 0 (0) | 14445 | 4261.7 | 119 | 7061 (0) | 0 | 392 |

## The server while it's loaded

| Heavy offered | Rust cores busy | Rust peak RSS MiB | Elixir cores busy | Elixir peak RSS MiB |
|---|---|---|---|---|
| none | 0.1 | 16 | 0.5 | 414 |
| 0.5× own | 3.9 | 97 | 5.9 | 606 |
| 0.9× own | 7.9 | 175 | 10.4 | 1443 |
| 1× own | 8.8 | 852 | 10.7 | 2012 |
| 1.25× own | 8.7 | 4500 | 10.3 | 3714 |
| 1.5× own | 8.5 | 5673 | 10 | 4751 |
| 2× own | 8.3 | 6092 | 9.7 | 6126 |
| 3× own | 8.6 | 6733 | 9.8 | 8265 |

## Recovery

After the heavy stream stops, seconds until the cheap stream's p99 stays within 2× its baseline (or +2 ms) with no failures:

- **ash-rust**: 9 s, 10 s (baseline p99 3.6 ms, 4.5 ms)
- **Ash**: 13 s, 14 s (baseline p99 4.3 ms, 8.6 ms)

Postgres, both desks and the driver share one machine, and the heavy request spends much of its time in Postgres, which costs the same on both sides; the figures compare how the desks treat a small request while a large one fills them, not their capacity. Whether the load is ever beyond the driver or the database rather than the desk is for the `server` and `driver late` columns of the run's log to say.
