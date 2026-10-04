# Performance Benchmarks: ash-rust vs. Ash Elixir

How `ash-rust` compares with [Ash](https://ash-hq.org/) in Elixir. There are two kinds of
comparison, and they answer different questions:

- **End to end** (the [supportdesk](../examples/supportdesk/README.md#benchmark) twin): the same
  application in both, served over HTTP from PostgreSQL, driven through the APIs real clients
  use (AshTypescript RPC, GraphQL, JSON). This is what an application sees.
- **Framework overhead** (the `helpdesk` micro-benchmarks): single actions and GraphQL
  queries against the in-memory data layers, with no database or network. These isolate the
  framework's own cost, so their ratios are far larger than an application will see.

---

## 1. Summary

End to end, ash-rust is **about 1.1–2.5x ahead** of Ash:

- **Reads:** 1.5–2.5x the throughput (16 clients, closed loop).
- **Writes:** 1.1–1.9x lower median latency at fixed request rates. Two of them (`route` over
  GraphQL and `bulk`) are within noise.
- **Server CPU:** about 3.5–10x less per operation.
- **Memory:** the Rust desk peaked at 15–27 MiB across the scenarios, against 373–504 MiB for Ash.

The gap is smaller than the in-memory micro-benchmarks suggest (6–10x) because, once a request
crosses HTTP and PostgreSQL, much of its time is spent in the database and the network, which
cost the same on both sides. The CPU and memory figures show more of the framework's
difference.

---

## 2. End-to-end results (supportdesk)

A full run on 2026-10-04: ash-rust at `3de168f` against Ash 3.34.0 (AshPostgres 2.14.0,
AshGraphql 1.12.0, AshTypescript 0.19.0, Phoenix 1.8.15), on an Apple M4 Max (16 cores,
128 GB) with PostgreSQL 16.15, Node 22.22 as the driver, and Rust 1.90.0. Both desks answered
parity (115 checks) and the smoke test (7) alike before anything was timed.

Reads ran 3 reps of 5 s per desk; writes ran 2 reps per desk, each on a fresh copy of the
seeded data. Each figure is the median over reps. **Ratio** is how many times better ash-rust
does: throughput for reads, p50 latency for writes. *Within noise* means within 5% or within
the spread between reps.

| Scenario | Over | Rust | Elixir | Ratio | Rust p50 / p99 ms | Elixir p50 / p99 ms | Server CPU s per 1k ops (Rust / Elixir) |
|---|---|---:|---:|---:|---|---|---|
| inbox | rpc | 7322/s | 4194/s | 1.75× | 1.89 / 6.76 | 3.49 / 9.98 | 0.34 / 1.47 |
| inbox | graphql | 6950/s | 3617/s | 1.92× | 2.15 / 5.46 | 4.26 / 9.42 | 0.56 / 2.03 |
| dashboard | rpc | 4054/s | 2132/s | 1.90× | 3.73 / 8.02 | 6.5 / 19.05 | 0.26 / 1.94 |
| dashboard | graphql | 3976/s | 2589/s | 1.54× | 3.83 / 8.08 | 5.95 / 11.2 | 0.36 / 2.5 |
| detail | rpc | 8300/s | 3356/s | 2.47× | 1.82 / 4 | 3.67 / 18.97 | 0.23 / 1.61 |
| detail | graphql | 4721/s | 2006/s | 2.35× | 2.85 / 10.76 | 6.89 / 23.26 | 0.39 / 2.83 |
| workflow | rpc | 10.91 ms | 19.38 ms | 1.78× | 10.91 / - | 19.38 / - | 0.75 / 7.3 |
| workflow | graphql | 14.08 ms | 18.14 ms | 1.29× | 14.08 / - | 18.14 / - | 1.1 / 8.4 |
| route | rpc | 6.47 ms | 12.41 ms | 1.92× | 6.47 / - | 12.41 / - | 0.65 / 5.25 |
| route | graphql | 9.52 ms | 12.92 ms | 1.36× *within noise* | 9.52 / - | 12.92 / - | 0.65 / 5.3 |
| counters | rpc | 4.03 ms | 6.2 ms | 1.54× | 4.03 / - | 6.2 / - | 0.31 / 2.34 |
| counters | graphql | 4.51 ms | 6.07 ms | 1.35× | 4.51 / - | 6.07 / - | 0.34 / 2.32 |
| edit races | json | 6.35 ms | 9.23 ms | 1.45× | 6.35 / - | 9.23 / - | 0.7 / 6.1 |
| bulk | json | 66.63 ms | 74.65 ms | 1.12× *within noise* | 66.63 / - | 74.65 / - | 6.5 / 25 |
| events | graphql | 5.38 ms | 8.54 ms | 1.59× | 5.38 / - | 8.54 / - | - |

Writes run at modest fixed rates (2–100 per second), so their p99 needs more samples than a
rep gives and is not reported.

**Caveats.**
- PostgreSQL, both desks and the load driver share one machine, so these figures compare the
  desks with each other; they are not capacity numbers.
- `detail` over GraphQL varied most between reps on both desks.
- `counters` views can lose a race under the ticket's optimistic lock, as `not_found`, on
  both desks.

The supportdesk README explains the scenarios and how the driver keeps the comparison fair,
and [GAPS.md](../examples/supportdesk/GAPS.md) lists where the two desks differ.

---

## 3. Framework overhead (in-memory micro-benchmarks)

These measure the framework alone: the `Helpdesk` domain (`Ticket` and `Representative`)
against `ash-memory` and `Ash.DataLayer.Ets`, and GraphQL queries over the same, with no
database, network or HTTP. They are useful for tracking ash-rust's own overhead over time, not
for predicting how much faster an application will be.

Measured 2026-09-18.

| Workload | Ash Elixir (ETS) | ash-rust (in-memory) | Ratio |
| :--- | :--- | :--- | :--- |
| `Ticket.open` (validate + changeset + write) | 26.63 µs (37.6k ips) | 3.23 µs (309.2k ips) | 8.2x |
| `Representative.create` (validate + write) | 23.74 µs (42.1k ips) | 2.47 µs (405.2k ips) | 9.6x |
| `Ticket.read` (filter `status == open`, 100 rows) | 208.63 µs (4.79k ips) | 32.79 µs (30.5k ips) | 6.4x |
| p99 latency (`Ticket.open`) | 59.04 µs | 7.67 µs | 7.7x lower |

### GraphQL (`ash_graphql` + Absinthe vs `ash-graphql`), in-memory

Median latency in parentheses.

| GraphQL Workload | Ash Elixir | ash-rust | Rust Advantage |
| :--- | :--- | :--- | :--- |
| Single record by ID | 2,850 ops/sec (311 µs) | **26,861 ops/sec (35.5 µs)** | **9.4x faster** |
| 100 tickets collection | 1,180 ops/sec (796 µs) | **4,471 ops/sec (216.5 µs)** | **3.8x faster** |
| Filtered & sorted (50) | 1,380 ops/sec (688 µs) | **7,932 ops/sec (119.7 µs)** | **5.8x faster** |
| Keyset pagination (first: 20) | 1,780 ops/sec (531 µs) | **6,942 ops/sec (137.0 µs)** | **3.9x faster** |
| DataLoader (100 tickets + author) | **760 ops/sec (1,358 µs)** | 664 ops/sec (1,419 µs) | **0.9x** (Elixir slightly ahead) |
| Mutation: `openTicket` | 5,800 ops/sec (156 µs) | **41,845 ops/sec (22.5 µs)** | **7.2x faster** |

### PostgreSQL data layer, single operations

[`benches/README.md`](../benches/README.md) has an older single-operation PostgreSQL
comparison (point writes, reads, aggregates, bulk ingestion, transactions). It was measured
before ash-postgres moved to tokio-postgres, and ranges from 0.7x to 2.3x.

### Micro-benchmark environment

- **Host Machine**: Apple MacBook Pro (Apple M4 Max, 16 CPU cores, 128 GB unified memory)
- **Operating System**: macOS 26.6.2
- **Rust Runtime**: Rust 1.90.0, release profile (`opt-level = 3`, LTO enabled)
- **Elixir Runtime**: Elixir 1.20.1 running on Erlang/OTP 29.0.2 (BEAM JIT enabled)
- **Code server**: Elixir suites switch to `:embedded` mode after warmup (production release default)
- **Framework Versions**: `ash-core 0.1.0` vs `ash 3.33.6` / `ash_graphql 1.12.0` (Hex packages)
- **Test Harnesses**: Standalone release runners (Rust) and Benchee 1.5.1 (Elixir)
- **Sampling Configuration**: Core 2.0 s warmup + 5.0 s sampling; GraphQL Elixir 1.0 s warmup + 3.0 s sampling; GraphQL Rust 0.5 s warmup + 2.0 s sampling

### Workload definitions

### Workload 1: `Ticket.open`
- **Pipeline**:
  1. Accepts input arguments (`subject: String`).
  2. Runs validations: `present(:subject)` and `string_length(:subject, min: 2)`.
  3. Executes changeset mutations: `set_attribute(:status, "open")` and sets actor relationship.
  4. Persists the record to the in-memory data layer (`ash-memory` vs `Ash.DataLayer.Ets`).
- **Elixir Implementation**: `Helpdesk.Support.open_ticket!("Printer is broken")`
- **Rust Implementation**: `customer.open_ticket("Printer is broken").await.unwrap()`

### Workload 2: `Representative.create`
- **Pipeline**:
  1. Accepts `name: String`.
  2. Validates presence and minimum string length.
  3. Persists to storage.
- **Elixir Implementation**: `Helpdesk.Support.create_representative!("Alice Smith")`
- **Rust Implementation**: `desk.create_representative("Alice Smith").await.unwrap()`

### Workload 3: `Ticket.read` (Filtered Query)
- **Pipeline**:
  1. Pre-populates 100 ticket records in memory.
  2. Builds query with filter `status == "open"`.
  3. Executes data layer query, materializes records into resource structs.
- **Elixir Implementation**: `Ash.Query.filter(Helpdesk.Support.Ticket, status == "open") |> Ash.read!()`
- **Rust Implementation**: `Ticket::query(&customer).filter(t::status.eq("open")).all().await.unwrap()`

### Workload 4: `load_aggregates`
- **Pipeline**:
  1. Runs query loading 3 concurrent aggregates: `ticket_count` (`count`), `open_ticket_count` (`count` with filter), and `has_tickets` (`exists`).
  2. Rust execution time: **~3.00 µs**.

---

## 4. Why ash-rust is ahead, and by how much

- **Native code and no per-process garbage collector.** The action pipeline (input casting,
  validations, changes, policies, the data layer call) does the same kind of work as Ash's,
  and is just as metadata-driven. Records move through it as maps of values
  (`FieldMap`), and the typed builders the macros generate sit on top. What differs is that it
  runs as compiled native code, not on the BEAM. That shows most clearly in CPU per operation
  (about 3.5–10x less, end to end) and in memory (about 20 MiB against about 450 MiB).
- **The database sets the floor.** A request that waits on PostgreSQL waits just as long on
  either side, so the end-to-end ratios are much smaller than the in-memory ones. Writes,
  which spend more of their time in transactions and round trips, show the smallest gaps.
- **Where Ash is level or ahead.** In the micro-benchmarks, DataLoader-batched GraphQL
  relationships are slightly faster in Ash, and the older PostgreSQL point read was faster in
  Ash. End to end, no scenario currently has ash-rust behind, but `bulk` and `route` over
  GraphQL are within noise.

---

## 5. Benchmark Codebase Structure

The benchmark assets are structured into three continuous testing components:

```
ash-rust/
├── benches/
│   ├── README.md               # Quick execution instructions
│   ├── compare.sh              # Unified dual-runner script
│   └── ash_elixir_bench.exs    # Self-contained Elixir Ash + Benchee suite
├── docs/
│   ├── benchmarks.md           # This comprehensive document
│   └── README.md               # Documentation root index
└── examples/
    ├── supportdesk/
    │   ├── bench/              # End-to-end driver (bench.ts) and scenarios
    │   └── README.md           # Scenarios, method, latest results
    ├── elixir/supportdesk/     # The Ash (Elixir) twin
    └── helpdesk/
        ├── Cargo.toml          # Configures Criterion [[bench]] target
        ├── examples/
        │   └── bench.rs        # Standalone Rust release throughput runner
        └── benches/
            └── helpdesk_bench.rs # Criterion statistical regression suite
```

---

## 6. Running the Benchmarks

### End to end (supportdesk)

```bash
cargo build --release -p supportdesk --bins
(cd examples/elixir/supportdesk && MIX_ENV=prod mix release --overwrite)
cd examples/supportdesk
node bench/bench.ts --fixture /tmp/fixture.json            # about 8–10 minutes
node bench/bench.ts --fixture /tmp/fixture.json --quick    # one short rep of each
```

Generate the fixture first with `target/release/fixture --out /tmp/fixture.json`. The
supportdesk README covers the prerequisites: Node 22.18+, `psql`, and a PostgreSQL to connect
to (`--pg`, by default `postgres://postgres:postgres@127.0.0.1:55434`).

### 1. Criterion Statistical Regression Suite

Run statistical benchmarking with automated baseline tracking:

```bash
cargo bench -p helpdesk
```

Criterion features:
- Samples each benchmark 100 times after warmup.
- Automatically calculates 95% confidence intervals and detects performance drift.
- Stores historical baselines in `target/criterion/`.
- Warns in red if code changes introduce regressions: `Performance has regressed: +15.3%`.
- Generates interactive SVG charts and HTML reports:
  ```bash
  open target/criterion/report/index.html
  ```

#### Fast Verification Mode
To quickly verify benchmark compilation and execution without full sampling:
```bash
cargo bench -p helpdesk -- --test
```

### 2. High-Throughput Release Runner

To run the custom standalone Rust micro-benchmark:
```bash
cargo run --release -p helpdesk --example bench
```

### 3. Elixir Ash Suite

The Elixir scripts call `AshBench.EmbeddedMode.enter!/0` after priming so `Code.ensure_loaded/1` misses do not walk the Mix.install code path. That is the same `:embedded` behavior a release uses.

To run the Elixir Ash benchmark with Benchee:
```bash
# Using mise:
mise exec elixir erlang -- elixir benches/ash_elixir_bench.exs

# Or with system Elixir:
elixir benches/ash_elixir_bench.exs
```

### 4. Back-to-Back Comparison Script

Run both suites consecutively on the same machine:
```bash
./benches/compare.sh
```

---

## 7. Continuous Integration (CI) Workflow

To catch performance regressions automatically in CI, add this job to `.github/workflows/bench.yml`:

```yaml
name: Benchmark Regression Check

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  benchmark:
    name: Criterion Benchmarks
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Run Criterion Benchmarks
        run: cargo bench -p helpdesk --bench helpdesk_bench -- --test
```
