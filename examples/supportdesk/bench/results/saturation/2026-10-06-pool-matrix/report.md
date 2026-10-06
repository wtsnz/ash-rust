# Pool matrix, 2026-10-06T02:57:02.005Z

ash-rust at `b950fe3a38` (uncommitted changes) against Ash (Elixir) on Apple M4 Max (16 cores), PostgreSQL 16.15 (Debian 16.15-1.pgdg12+2). Machine load at the start: 16.11 9.83 7.11.

Each row is one 30 s test (after 3 s of warm-up) of the same two streams as the ramp: the cheap one, 500/s, and the heavy one at the rate named, the same for every configuration. Rust's pool either queues without limit (its default) or fails a statement that has waited 100 ms (`PoolSettings::wait_timeout`). Elixir's either sheds, as Ecto's `queue_target` of 50 ms does (the default), or has its `queue_target` and `queue_interval` set to 60 s, so it queues. Pool sizes are 20 and 40. One rep each: read the differences between configurations as indications, not as measured to the noise. Postgres columns are sampled during the window: its connections in use by the desk's database (of those open), and its container's CPU as a share of one core.

| Heavy offered | Configuration | Pool | Cheap p50 / p99 ms | Cheap errors | Heavy answered/s | Heavy errors | Heavy not sent | Heavy p50 ms | Server cores | Peak RSS MiB | PG active / open | PG container CPU % |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---|---:|
| 800/s | Rust, queues (no wait limit) | 20 | 1454.4 / 1746.7 | 0 (0%) | 632 | 0 (0%) | 2603 | 4436.5 | 8.5 | 6606 | 2.3 / 20 | 285 |
| 800/s | Elixir, sheds (Ecto's default) | 20 | 109.2 / 906.3 | 4967 (33%) | 117 | 20528 (86%) | 0 | 423 | 9.7 | 7595 | 3.5 / 19 | 309 |
| 800/s | Rust, 100 ms wait limit | 20 | 99.4 / 115.7 | 846 (6%) | 628 | 5185 (22%) | 0 | 311.2 | 8.4 | 853 | 2.4 / 20 | 280 |
| 800/s | Elixir, queues (queue_target 60 s) | 20 | 3842.7 / 6062.2 | 0 (0%) | 86 | 8708 (36%) | 14634 | 9447.6 | 10.5 | 23261 | 1.9 / 20 | 239 |
| 800/s | Rust, queues (no wait limit) | 40 | 1527 / 1857.8 | 0 (0%) | 625 | 0 (0%) | 4014 | 4573.6 | 8.2 | 6549 | 1.5 / 40 | 296 |
| 800/s | Elixir, sheds (Ecto's default) | 40 | 115.8 / 1077.6 | 5152 (34%) | 116 | 20611 (86%) | 0 | 463.8 | 9.4 | 7561 | 6.1 / 37.1 | 330 |
| 800/s | Rust, 100 ms wait limit | 40 | 101.6 / 132.2 | 543 (4%) | 689 | 3315 (14%) | 0 | 320.5 | 8.9 | 1015 | 2.1 / 40 | 260 |
| 800/s | Elixir, queues (queue_target 60 s) | 40 | 3840.8 / 5778.1 | 0 (0%) | 72 | 8928 (37%) | 14856 | 9569.8 | 10.9 | 22909 | 2.4 / 40 | 271 |
| 1600/s | Rust, queues (no wait limit) | 20 | 1470.7 / 1689.1 | 0 (0%) | 685 | 0 (0%) | 27723 | 4361.8 | 8.6 | 7533 | 3.2 / 20 | 291 |
| 1600/s | Elixir, sheds (Ecto's default) | 20 | 108.1 / 1079.6 | 7770 (52%) | 22 | 45925 (93%) | 3001 | 387.7 | 8.9 | 10715 | 4.3 / 15.1 | 392 |
| 1600/s | Rust, 100 ms wait limit | 20 | 102.4 / 116 | 3324 (22%) | 489 | 33246 (69%) | 0 | 323 | 7.9 | 1024 | 4.2 / 20 | 361 |
| 1600/s | Rust, queues (no wait limit) | 40 | 1441.3 / 1803.3 | 0 (0%) | 696 | 0 (0%) | 27458 | 4245 | 8.6 | 9822 | 3.4 / 40 | 288 |
| 1600/s | Elixir, sheds (Ecto's default) | 40 | 111.6 / 1133.1 | 8049 (54%) | 22 | 45888 (93%) | 3001 | 409.8 | 8.7 | 10589 | 8.1 / 32.5 | 419 |
| 1600/s | Rust, 100 ms wait limit | 40 | 106.1 / 127.5 | 3213 (21%) | 554 | 31284 (65%) | 0 | 335.1 | 8.1 | 1221 | 2.8 / 40 | 364 |
