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
     - Measures `Ticket.open`, `Representative.create`, and 100-record filtered reads using Ash 3.0 + `Ash.DataLayer.Ets`.
     - Run: `mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs`
   - **GraphQL API Benchmark**: `benches/ash_graphql_elixir_bench.exs`
     - Measures `getTicket` single record, 100-record collection, filtered & sorted queries, keyset pagination, DataLoader nested relationships, and `openTicket` mutations using `ash_graphql` + `absinthe` + ETS.
     - Run: `mise exec elixir erlang -- elixir benches/ash_graphql_elixir_bench.exs`
   - **PostgreSQL Benchmark**: `benches/ash_postgres_elixir_bench.exs`
     - Measures point writes, reads by ID, filtered/sorted queries, correlated aggregates, bulk ingestion, and `Ash.transaction` against PostgreSQL using `ash_postgres` + `Ecto` + `Postgrex`.
     - Run: `mise exec elixir erlang -- elixir benches/ash_postgres_elixir_bench.exs`

---

## Results

These suites measure the framework's own overhead, mostly in memory, and track it over time.
For how ash-rust compares with Ash on a real application, see
[docs/benchmarks.md](../docs/benchmarks.md).

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
