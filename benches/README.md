# Ash Benchmarks: Rust vs. Elixir

This directory contains benchmarking suites to monitor `ash-rust` performance over time and compare it continuously against canonical Ash in Elixir.

## Benchmark Components

1. **Criterion Regression Suite (`cargo bench`)**
   - Location: `examples/helpdesk/benches/helpdesk_bench.rs`
   - Purpose: Industry-standard statistical benchmarking with automated regression detection and HTML reports.
   - Measures:
     - `actions/ticket_open_memory`: Action invocation + validation + changeset + in-memory data layer.
     - `actions/representative_create_memory`: Resource creation + validation.
     - `actions/ticket_open_sqlite`: Action invocation + validation + SQL generation + SQLite execution.
     - `queries/filter_status_open_100_memory`: Query pipeline filtering 100 rows in-memory.
     - `queries/filter_status_open_100_sqlite`: SQL `SELECT` with `WHERE` filter across 100 SQLite rows.
     - `aggregates/load_aggregates_memory`: Calculating `count`, `open_count`, and `exists` aggregates.
     - `aggregates/load_aggregates_sqlite`: Correlated SQL subqueries for aggregates.

2. **Comparative Benchmark Script (`benches/compare.sh`)**
   - Runs the Rust release runner (`examples/helpdesk/examples/bench.rs`) alongside the Elixir Ash runner (`benches/ash_elixir_bench.exs`) on the exact same hardware to track the speedup multiplier over time.

3. **Ash Elixir Benchmark (`benches/ash_elixir_bench.exs`)**
   - Canonical Ash 3.0 implementation of the same Helpdesk resources using `Ash.DataLayer.Ets` and Benchee for memory and throughput tracking.

---

## How to Run

### 1. Statistical Regression Testing (Criterion)

Run the Criterion suite:
```bash
cargo bench -p helpdesk
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
```

### 2. Side-by-Side Comparison with Elixir

Run both suites back-to-back:
```bash
./benches/compare.sh
```

Or run the Elixir suite standalone:
```bash
mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs
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
