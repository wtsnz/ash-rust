# Ash Benchmarks: Rust vs. Elixir

This directory contains benchmarking suites to monitor `ash-rust` performance over time and compare it continuously against canonical Ash in Elixir.

## Benchmark Components

1. **Criterion Regression Suites (`cargo bench`)**
   - **Helpdesk Core Suite**: `examples/helpdesk/benches/helpdesk_bench.rs`
     - Measures raw action invocations, changeset pipelines, memory and SQLite filtering, and correlated aggregate queries.
     - Command: `cargo bench -p helpdesk`
   - **GraphQL API Suite**: `crates/ash-graphql/benches/graphql_bench.rs`
     - Measures dynamic GraphQL schema reflection, single record queries by ID, 100-record collections, filtered & sorted queries, keyset pagination, DataLoader N+1 relationship batching, GraphQL mutations, Axum HTTP POST `/graphql` roundtrips, and SQLite data layer queries.
     - Command: `cargo bench -p ash-graphql --bench graphql_bench --features axum`

2. **Fast Standalone Benchmark Runners**
   - **Core Engine Runner**: `examples/helpdesk/examples/bench.rs`
     - Run: `cargo run --release -p helpdesk --example bench`
   - **GraphQL API Runner**: `crates/ash-graphql/examples/bench_graphql.rs`
     - Run: `cargo run --release -p ash-graphql --example bench_graphql --features axum`
     - Measures iterations, throughput (ops/sec), average, median (p50), p95, and p99 latencies for realistic web workloads.
   - **PostgreSQL DataLayer Runner**: `crates/ash-postgres/examples/bench_postgres.rs`
     - Run: `cargo run --release -p ash-postgres --example bench_postgres`
     - Measures point writes (`RETURNING *`), point reads by ID, filtered & sorted queries, correlated aggregate subqueries, vectorized bulk ingestion (`compile_bulk_insert`), and multi-step transactions against PostgreSQL.

3. **Comparative Benchmark Scripts**
   - `benches/compare.sh`: Runs both core in-memory and GraphQL suites for Rust and Elixir back-to-back. (Pass `--postgres` to include PostgreSQL).
   - `benches/bench_postgres.sh`: Spawns or connects to the PostgreSQL Docker container, runs the Rust `ash-postgres` suite, and runs the Elixir `AshPostgres` suite for direct side-by-side comparison.

4. **Ash Elixir Benchmark Suites (Benchee)**
   After priming each workload, the suites switch the BEAM code server to `:embedded` mode (the production release default) so optional-module lookups do not walk the Mix.install code path.
   - **Core Engine Benchmark**: `benches/ash_elixir_bench.exs`
     - Measures `Ticket.open`, `Representative.create`, 100-record filtered reads and `load_aggregates` using Ash 3.0 + `Ash.DataLayer.Ets`, through the helpdesk's policies.
     - Run: `mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs`
   - **GraphQL API Benchmark**: `benches/ash_graphql_elixir_bench.exs`
     - Measures `getTicket` single record, 100-record collection, filtered & sorted queries, keyset pagination, DataLoader nested relationships, and `openTicket` mutations using `ash_graphql` + `absinthe` + ETS.
     - Run: `mise exec elixir erlang -- elixir benches/ash_graphql_elixir_bench.exs`
   - **PostgreSQL Benchmark**: `benches/ash_postgres_elixir_bench.exs`
     - Measures point writes, reads by ID, filtered/sorted queries, correlated aggregates, bulk ingestion, and `Ash.transaction` against PostgreSQL using `ash_postgres` + `Ecto` + `Postgrex`.
     - Run: `mise exec elixir erlang -- elixir benches/ash_postgres_elixir_bench.exs`

5. **The Elixir twins' benchmarks**
   - **Helpdesk with its policies**: `examples/elixir/helpdesk/bench/core.exs`
     - The workloads of `examples/helpdesk/examples/bench.rs` on the full Elixir desk, with the
       policies the Rust desk runs, as the Benchee script above does, on ETS and on SQLite,
       plus `load_aggregates` on ETS. AshSqlite serves no resource aggregates.
     - Run: `cd examples/elixir/helpdesk && mise exec elixir erlang -- mix run bench/core.exs`
   - **Cybercab**: `examples/benchmarks/cybercab` drives the Rust and Elixir servers with the same load.
   - **Supportdesk**: `examples/supportdesk/bench` drives both desks through their real clients.
   - Kanban and astro-helpdesk have twins ([`examples/GAPS.md`](../examples/GAPS.md)) but no benchmark.

---

## Performance Comparison: Rust (`ash-rust`) vs. Elixir (`Ash 3.0`)

Measured 2026-10-05 (core actions and PostgreSQL) on Apple M4 Max (16 cores, 128GB RAM), with other applications open (load average 6-10), so read the ratios rather than the absolute figures; the GraphQL figures are those of 2026-09-18. Elixir 1.20.1 / OTP 29.0.5 with the code server in `:embedded` mode after warmup. Rust 1.90.0 release. Ash 3.34.4 vs `ash-core` 0.1.0. Every figure is the median of several runs (core: 6 Rust, 4 Elixir; PostgreSQL: 8 Rust, 5 Elixir).

### Core Engine & Actions

Both sides run through the helpdesk's policies: `open` needs an actor, and a read is filtered to what the reader may read (the customer who opened the tickets).

| Workload | Ash Elixir Throughput | Ash Elixir Median Latency | ash-rust Throughput | ash-rust Median Latency | Speedup |
|:---|:---:|:---:|:---:|:---:|:---:|
| `Ticket.open` (Policy + Validation + Action) | 28,270 ips | 33.8 µs | **228,073 ips** | **4.08 µs** | **~8.1x faster** |
| `Representative.create` (Action) | 37,985 ips | 25.0 µs | **279,831 ips** | **3.33 µs** | **~7.4x faster** |
| `Ticket.read` (100 records filter) | 3,215 ips | 296.3 µs | **25,715 ips** | **37.7 µs** | **~8.0x faster** |
| `load_aggregates` (3 aggregates, 20 tickets) | 2,100 ips | 457.6 µs | **38,344 ips** | **26.1 µs** | **~18.3x faster** |

### GraphQL API Layer

| GraphQL Workload | Ash Elixir (`ash_graphql` + Absinthe) | Ash Rust (`ash-graphql`) | Rust Speedup Multiplier |
|:---|:---:|:---:|:---:|
| **1. Single Record by ID** | 2,850 ops/sec (311 µs) | **26,861 ops/sec (35.5 µs)** | **~9.4x faster** (8.8x lower latency) |
| **2. 100 Tickets Collection** | 1,180 ops/sec (796 µs) | **4,471 ops/sec (216.5 µs)** | **~3.8x faster** (3.7x lower latency) |
| **3. Filtered & Sorted (50 items)** | 1,380 ops/sec (688 µs) | **7,932 ops/sec (119.7 µs)** | **~5.8x faster** (5.8x lower latency) |
| **4. Keyset Pagination (first: 20)** | 1,780 ops/sec (531 µs) | **6,942 ops/sec (137.0 µs)** | **~3.9x faster** (3.9x lower latency) |
| **5. DataLoader (100 Tickets + Author)** | **760 ops/sec (1,358 µs)** | 664 ops/sec (1,419 µs) | **~0.9x** (Elixir slightly ahead) |
| **6. Mutation: `openTicket`** | 5,800 ops/sec (156 µs) | **41,845 ops/sec (22.5 µs)** | **~7.2x faster** (6.9x lower latency) |

### PostgreSQL Data Layer (`ash-postgres` vs `ash_postgres` + Ecto)

PostgreSQL 16 (`pgvector/pgvector:pg16`) in Docker on the same machine. Writes are bound by the VM's fsync and vary by about 2x between runs, so the table gives each side's median throughput and median latency, with the range across runs below it.

| PostgreSQL Workload | Ash Elixir (`ash_postgres` + Ecto) | Ash Rust (`ash-postgres`, tokio-postgres) | Rust Speedup Multiplier |
|:---|:---:|:---:|:---:|
| **1. Point Write (`RETURNING *`)** | 766 ops/sec (1,120 µs) | **958 ops/sec (1,010 µs)** | **~1.25x faster** (1.11x lower latency) |
| **2. Point Read (`id` PK Lookup)** | 6,960 ops/sec (137 µs) | **11,969 ops/sec (81 µs)** | **~1.72x faster** (1.70x lower latency) |
| **3. Filtered & Sorted (50 items)** | 1,250 ops/sec (770 µs) | **3,390 ops/sec (304 µs)** | **~2.71x faster** (2.53x lower latency) |
| **4. Correlated Aggregates (Subqueries)** | 1,250 ops/sec (670 µs) | **4,137 ops/sec (251 µs)** | **~3.31x faster** (2.67x lower latency) |
| **5. Bulk Ingestion (100 Tickets)** | 137 ops/sec (6,650 µs) | **311 ops/sec (2,934 µs)** | **~2.27x faster** (2.27x lower latency) |
| **6. Transactional Workflow (BEGIN/COMMIT)** | 750 ops/sec (1,200 µs) | 646 ops/sec (1,144 µs) | ~0.9x (level: the runs' ranges overlap) |

Range of throughput across runs, Elixir / Rust, in ops/sec: write 510-990 / 520-1,527; read 6,220-6,990 / 11,290-12,748; filtered 599-2,010 / 1,959-4,778; aggregates 1,190-1,880 / 2,713-6,055; bulk 105-149 / 239-377; transaction 660-950 / 328-1,066.

---

## How to Run

### 1. Statistical Regression Testing (Criterion)

Run the Criterion suites:
```bash
cargo bench -p helpdesk
cargo bench -p ash-graphql --bench graphql_bench --features axum
cargo bench -p ash-postgres
```

Criterion automatically:
- Samples iterations with statistical warmups and outlier filtering.
- Compares timings against the previous baseline saved in `target/criterion/`.
- Warns if any code change introduced a performance regression (e.g. `Performance has regressed: +14.2%`).
- Generates interactive SVG charts and HTML reports:
  ```bash
  open target/criterion/report/index.html
  ```

To quickly verify the benchmark suites without a full sampling run:
```bash
cargo bench -p helpdesk -- --test
cargo bench -p ash-graphql --bench graphql_bench --features axum -- --test
cargo bench -p ash-postgres -- --test
```

### 2. Side-by-Side Comparison with Elixir

Run in-memory & GraphQL suites back-to-back:
```bash
./benches/compare.sh
```

Run PostgreSQL benchmarks against Docker (auto-provisions container):
```bash
./benches/bench_postgres.sh
```

Or run all suites including PostgreSQL:
```bash
./benches/compare.sh --postgres
```

---

## Continuous Integration (CI)

To catch performance regressions in GitHub Actions:

```yaml
name: Benchmark

on:
  push:
    branches: [main]
  pull_request:

jobs:
  bench:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run benchmarks
        run: cargo bench -p helpdesk
```
