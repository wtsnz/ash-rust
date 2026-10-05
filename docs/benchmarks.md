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

The gap is smaller than in-memory micro-benchmarks suggest because, once a request crosses HTTP
and PostgreSQL, much of its time is spent in the database and the network, which cost the same
on both sides. The CPU and memory figures show more of the framework's
difference.

---

## 2. End-to-end results (supportdesk)

A full run on 2026-10-04: ash-rust against Ash 3.34.0 (AshPostgres 2.14.0,
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
database, network or HTTP. They track ash-rust's own overhead (Criterion keeps a baseline and
flags regressions); they don't predict how much faster an application will be. See
[Running the Benchmarks](#6-running-the-benchmarks) to measure them.

#### Workload definitions

#### Workload 1: `Ticket.open`
- **Pipeline**:
  1. Accepts input arguments (`subject: String`).
  2. Runs validations: `present(:subject)` and `string_length(:subject, min: 2)`.
  3. Executes changeset mutations: `set_attribute(:status, "open")` and sets actor relationship.
  4. Persists the record to the in-memory data layer (`ash-memory` vs `Ash.DataLayer.Ets`).
  Both run through the desk's policies: `open` needs an actor.
- **Elixir Implementation**: `Helpdesk.Support.open_ticket!("Printer is broken", actor: customer)`
- **Rust Implementation**: `customer.open_ticket("Printer is broken").await.unwrap()`

#### Workload 2: `Representative.create`
- **Pipeline**:
  1. Accepts `name: String`.
  2. Validates presence and minimum string length.
  3. Persists to storage.
- **Elixir Implementation**: `Helpdesk.Support.create_representative!("Alice Smith")`
- **Rust Implementation**: `desk.create_representative("Alice Smith").await.unwrap()`

#### Workload 3: `Ticket.read` (Filtered Query)
- **Pipeline**:
  1. Pre-populates 100 ticket records in memory.
  2. Builds query with filter `status == "open"`.
  3. Executes data layer query, materializes records into resource structs.
  The reader is the customer who opened the tickets: the read policy filters to the tickets they may read.
- **Elixir Implementation**: `Ash.Query.filter(Helpdesk.Support.Ticket, status == "open") |> Ash.read!(actor: reader)`
- **Rust Implementation**: `Ticket::query(&customer).filter(t::status.eq("open")).all().await.unwrap()`

#### Workload 4: `load_aggregates`
- **Pipeline**:
  1. Pre-populates a representative with 20 assigned tickets.
  2. Runs a query loading 3 aggregates: `ticket_count` (`count`), `open_ticket_count` (`count` with filter), and `has_tickets` (`exists`), as the customer who opened the tickets: an aggregate counts what its reader may read.
- **Elixir Implementation**: `Ash.Query.load(Helpdesk.Support.Representative, [:ticket_count, :open_ticket_count, :has_tickets]) |> Ash.read!(actor: reader)`
- **Rust Implementation**: `Representative::query(&customer).aggregate(r::ticket_count).aggregate(r::open_ticket_count).aggregate(r::has_tickets).all().await.unwrap()`

Both sides read through the helpdesk's policies, as the Rust desk always did. Until 2026-10-05 the
Elixir script modelled a ticket with no policies, and the Rust `load_aggregates` benchmark read
as no one, so it counted no tickets (see [`examples/GAPS.md`](../examples/GAPS.md)).

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
- **Where the gap is smallest.** No scenario has ash-rust behind, but `bulk` and `route` over
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

### Every benchmark, Rust and Elixir

| Layer | Rust | Elixir |
| :--- | :--- | :--- |
| Core actions | `examples/helpdesk` (`examples/bench.rs`, Criterion) | `benches/ash_elixir_bench.exs` (ETS); `examples/elixir/helpdesk/bench/core.exs` (the Elixir twin, ETS and SQLite) |
| Core aggregates | Criterion `load_aggregates` | `benches/ash_elixir_bench.exs` and `bench/core.exs` (ETS; AshSqlite has none) |
| GraphQL | `crates/ash-graphql/examples/bench_graphql.rs` | `benches/ash_graphql_elixir_bench.exs` |
| Postgres | `crates/ash-postgres/examples/bench_postgres.rs` | `benches/ash_postgres_elixir_bench.exs` |
| Cybercab, over HTTP and WebSocket | `examples/benchmarks/cybercab` | the same driver, against `examples/elixir/cybercab` |
| Supportdesk, through the generated clients | `examples/supportdesk/bench` | the same driver, against `examples/elixir/supportdesk` |

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
