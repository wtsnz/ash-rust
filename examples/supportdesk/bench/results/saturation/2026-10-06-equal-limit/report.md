# Equal wait limit, 2026-10-06T03:21:47.982Z

ash-rust at `eacd3d110d` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), PostgreSQL 16.15 (Debian 16.15-1.pgdg12+2). Machine load at the start: 6.21 10.46 13.58.

Each row is one 30 s test (after 3 s of warm-up) of the same two streams as the ramp: the cheap one, 500/s, and the heavy one at the rate named, the same for every configuration, on pools of 20 connections. Both desks are given the same limit on how long a request waits for a connection: Rust's is `PoolSettings::wait_timeout`, which fails a statement that has waited that long. Ecto has no hard limit: once its pool has been slow for an interval it drops what has waited past twice its `queue_target`, so Elixir is given a `queue_target` of half the limit and a `queue_interval` of 100 ms. One rep each: read the differences as indications. Postgres columns are sampled during the window: its connections in use by the desk's database (of those open), and its container's CPU as a share of one core.

| Heavy offered | Configuration | Pool | Cheap p50 / p99 ms | Cheap errors | Heavy answered/s | Heavy p50 / p99 ms | Heavy errors | Heavy not sent | Server cores | Peak RSS MiB | PG active / open | PG container CPU % |
|---|---|---:|---|---:|---:|---|---:|---:|---:|---:|---|---:|
| 400/s | Rust, 100 ms wait limit | 20 | 0.9 / 6.6 | 0 (0%) | 400 | 16.4 / 31.6 | 0 (0%) | 0 | 4.7 | 122 | 1.7 / 20 | 148 |
| 400/s | Elixir, 100 ms (queue_target 50) | 20 | 106.4 / 491.9 | 2238 (15%) | 174 | 432.6 / 1341.3 | 6760 (56%) | 0 | 9.4 | 3369 | 2.8 / 19.3 | 254 |
| 400/s | Rust, 250 ms wait limit | 20 | 0.8 / 12.3 | 0 (0%) | 400 | 16.2 / 54.5 | 0 (0%) | 0 | 4.6 | 196 | 1.7 / 20 | 154 |
| 400/s | Elixir, 250 ms (queue_target 125) | 20 | 241.2 / 1058.2 | 1959 (13%) | 195 | 803 / 1625.9 | 6049 (50%) | 0 | 9.9 | 4057 | 3.1 / 19.5 | 268 |
| 400/s | Rust, 1000 ms wait limit | 20 | 0.9 / 11.9 | 0 (0%) | 400 | 16.2 / 51.8 | 0 (0%) | 0 | 4.6 | 164 | 1.7 / 20 | 149 |
| 400/s | Elixir, 1000 ms (queue_target 500) | 20 | 913.3 / 1765.6 | 2177 (15%) | 186 | 2726.9 / 3438.7 | 6313 (53%) | 0 | 9.7 | 8392 | 3.1 / 19.2 | 273 |
| 800/s | Rust, 100 ms wait limit | 20 | 99.5 / 116.8 | 1032 (7%) | 591 | 311.2 / 341.3 | 6150 (26%) | 0 | 8 | 1521 | 3.1 / 20 | 277 |
| 800/s | Elixir, 100 ms (queue_target 50) | 20 | 108.5 / 762.2 | 5291 (35%) | 62 | 403.2 / 1034.7 | 22100 (92%) | 0 | 8.4 | 2792 | 4.4 / 18.5 | 340 |
| 800/s | Rust, 250 ms wait limit | 20 | 247.6 / 269.1 | 971 (6%) | 598 | 751.3 / 788.1 | 6008 (25%) | 0 | 8.3 | 4199 | 2.9 / 20 | 275 |
| 800/s | Elixir, 250 ms (queue_target 125) | 20 | 253.8 / 948.3 | 4380 (29%) | 92 | 833.2 / 1528.9 | 21139 (88%) | 0 | 9 | 3927 | 3.6 / 17.8 | 322 |
| 800/s | Rust, 1000 ms wait limit | 20 | 963.4 / 1018.6 | 749 (5%) | 597 | 2884.6 / 3028.6 | 4612 (19%) | 0 | 8.3 | 7602 | 2.9 / 20 | 288 |
| 800/s | Elixir, 1000 ms (queue_target 500) | 20 | 972.4 / 1993.8 | 4037 (27%) | 110 | 3013.1 / 3805.4 | 20202 (84%) | 0 | 9.6 | 9718 | 4.4 / 18.9 | 330 |
| 1600/s | Rust, 100 ms wait limit | 20 | 102.6 / 116 | 3435 (23%) | 467 | 323.8 / 340.4 | 33902 (71%) | 0 | 7.4 | 1627 | 4.3 / 20 | 364 |
| 1600/s | Elixir, 100 ms (queue_target 50) | 20 | 109 / 724 | 8160 (54%) | 13 | 399.6 / 870.1 | 47546 (99%) | 0 | 8.3 | 2444 | 5.4 / 16.4 | 382 |
| 1600/s | Rust, 250 ms wait limit | 20 | 252.2 / 266.4 | 3314 (22%) | 500 | 772.6 / 789.8 | 32829 (68%) | 0 | 8 | 5062 | 3.4 / 20 | 360 |
| 1600/s | Elixir, 250 ms (queue_target 125) | 20 | 258.8 / 741.5 | 8039 (54%) | 12 | 859.8 / 1092.2 | 47434 (99%) | 0 | 8.3 | 2995 | 4.9 / 16.5 | 372 |
| 1600/s | Rust, 1000 ms wait limit | 20 | 983.3 / 1017.8 | 2091 (14%) | 545 | 2919.4 / 3021.7 | 18630 (39%) | 13223 | 8 | 9733 | 2.4 / 20 | 339 |
| 1600/s | Elixir, 1000 ms (queue_target 500) | 20 | 1009.1 / 1371.7 | 7550 (50%) | 16 | 3139 / 3432.5 | 46804 (98%) | 0 | 9.1 | 7409 | 5.7 / 16.1 | 404 |
