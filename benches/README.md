# Ash Benchmarks: Rust vs. Elixir

This directory contains benchmarking suites to monitor `ash-rust` performance over time and compare it continuously against canonical Ash in Elixir.

## Benchmark Components

1. **Criterion Regression Suites (`cargo bench`)**
   - **Helpdesk Core Suite**: `examples/helpdesk/benches/helpdesk_bench.rs`
     - Measures raw action invocations, changeset pipelines, memory and SQLite filtering, and correlated aggregate queries.
     - Command: `cargo bench -p helpdesk`
   - **GraphQL API Suite**: `crates/ash-graphql/benches/graphql_bench.rs`
     - Measures dynamic GraphQL schema reflection, single record queries by ID, 100-record collections, filtered & sorted queries, Relay keyset pagination, DataLoader N+1 relationship batching, GraphQL mutations, Axum HTTP POST `/graphql` roundtrips, and SQLite data layer queries.
     - Command: `cargo bench -p ash-graphql --bench graphql_bench --features axum`

2. **Fast Standalone Benchmark Runners**
   - **Core Engine Runner**: `examples/helpdesk/examples/bench.rs`
     - Run: `cargo run --release -p helpdesk --example bench`
   - **GraphQL API Runner**: `crates/ash-graphql/examples/bench_graphql.rs`
     - Run: `cargo run --release -p ash-graphql --example bench_graphql --features axum`
     - Measures iterations, throughput (ops/sec), average, median (p50), p95, and p99 latencies for realistic web workloads.

3. **Comparative Benchmark Script (`benches/compare.sh`)**
   - Runs both Rust release runners alongside both canonical Elixir Ash runners on the exact same hardware to track performance and speedup multipliers over time.

4. **Ash Elixir Benchmark Suites (Benchee)**
   - **Core Engine Benchmark**: `benches/ash_elixir_bench.exs`
     - Measures `Ticket.open`, `Representative.create`, and 100-record filtered reads using Ash 3.0 + `Ash.DataLayer.Ets`.
     - Run: `mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs`
   - **GraphQL API Benchmark**: `benches/ash_graphql_elixir_bench.exs`
     - Measures `getTicket` single record, 100-record collection, filtered & sorted queries, keyset pagination, DataLoader nested relationships, and `openTicket` mutations using `ash_graphql` + `absinthe` + ETS.
     - Run: `mise exec elixir erlang -- elixir benches/ash_graphql_elixir_bench.exs`

---

## Performance Comparison: Rust (`ash-rust`) vs. Elixir (`Ash 3.0`)

Measured on Apple Silicon M-Series (10 cores, 32GB RAM):

### Core Engine & Actions

| Workload | Ash Elixir Throughput | Ash Elixir Median Latency | ash-rust Throughput | ash-rust Median Latency | Speedup |
|:---|:---:|:---:|:---:|:---:|:---:|
| `Ticket.open` (Validation + Action) | 25,210 ips | 37.3 µs | 31,140 ips | 31.0 µs | **~1.2x faster** |
| `Representative.create` (Action) | 26,370 ips | 34.4 µs | ~32,000 ips | ~31 µs | **~1.2x faster** |
| `Ticket.read` (100 records filter) | 2,740 ips | 340.1 µs | ~2,800 ips | ~350 µs | **~1.0x parity** |

### GraphQL API Layer

| GraphQL Workload | Ash Elixir (`ash_graphql` + Absinthe) | Ash Rust (`ash-graphql`) | Rust Speedup Multiplier |
|:---|:---:|:---:|:---:|
| **1. Single Record by ID** | 1,440 ops/sec (580 µs) | **14,895 ops/sec (64.8 µs)** | **~10.3x faster** (8.9x lower latency) |
| **2. 100 Tickets Collection** | 900 ops/sec (1,040 µs) | **2,574 ops/sec (377 µs)** | **~2.8x faster** (2.8x lower latency) |
| **3. Filtered & Sorted (50 items)** | 800 ops/sec (1,180 µs) | **4,514 ops/sec (217 µs)** | **~5.6x faster** (5.4x lower latency) |
| **4. Keyset Pagination (first: 20)** | 1,170 ops/sec (830 µs) | **3,733 ops/sec (259 µs)** | **~3.2x faster** (3.2x lower latency) |
| **5. DataLoader (100 Tickets + Author)** | 670 ops/sec (1,460 µs) | **576 ops/sec (1,675 µs)** | **~1.0x parity** |
| **6. Mutation: `openTicket`** | 3,550 ops/sec (240 µs) | **31,140 ops/sec (31.0 µs)** | **~8.8x faster** (7.7x lower latency) |

---

## How to Run

### 1. Statistical Regression Testing (Criterion)

Run the Criterion suite:
```bash
cargo bench -p helpdesk
cargo bench -p ash-graphql --bench graphql_bench --features axum
```

Criterion automatically:
- Samples iterations with statistical warmups and outlier filtering.
- Compares timings against the previous baseline saved in `target/criterion/`.
- Warns if any code change introduced a performance regression (e.g. `Performance has regressed: +14.2%`).
- Generates interactive SVG charts and HTML reports:
  ```bash
  open target/criterion/report/index.html
  ```

To quickly verify the benchmark suite without a full sampling run:
```bash
cargo bench -p helpdesk -- --test
cargo bench -p ash-graphql --bench graphql_bench --features axum -- --test
```

### 2. Side-by-Side Comparison with Elixir

Run all suites back-to-back:
```bash
./benches/compare.sh
```

Or run individual Elixir suites standalone:
```bash
mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs
mise exec elixir erlang -- elixir benches/ash_graphql_elixir_bench.exs
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
