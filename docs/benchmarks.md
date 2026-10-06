# Performance Benchmarks: ash-rust vs. Ash Elixir

How `ash-rust` compares with [Ash](https://ash-hq.org/) in Elixir. There are three kinds of
comparison, and they answer different questions:

- **End to end** (the [supportdesk](../examples/supportdesk/README.md#benchmark) twin): the same
  application in both, served over HTTP from PostgreSQL, driven through the APIs real clients
  use (AshTypescript RPC, GraphQL, JSON). This is what an application sees.
- **Under saturation** (the supportdesk twin again): what each desk does to a small request
  while large ones fill it, and what it does with load beyond its capacity.
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

Under saturation the two desks fail differently (section 2, below). ash-rust has about 2.6x
Ash's capacity for a large read, and keeps a small request quick, with no errors, up to about
90% of it; past that it queues, so every request waits, memory grows with the queue, and
nothing is refused. Ash starts shedding at 100% of its capacity: its connection pool drops
requests that have waited about 100 ms, so a small request that succeeds waits a steady
~110 ms and many fail (28% at twice its capacity), and what it answers falls as the load
grows. Neither the pool nor Postgres was the limit (doubling the pool changed nothing), and no scheduler was. Take the database out and make
the load CPU, and the BEAM's preemption shows: Ash holds the small request at about 8 ms (p99
~40 ms) however much heavy work is offered, while ash-rust, whose capacity is 4.7 times higher,
is faster until it saturates (about 85% of its capacity) and then lets the small request wait
behind the heavy ones (about 200 ms, p99 about 0.5 s). Keeping heavy work from queuing ahead of light work (an admission
limit, or a runtime of its own) is how ash-rust gets the same isolation.

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

### Under saturation (supportdesk)

`bench/saturation.ts` sends a cheap stream (one ticket by id, 500 a second) and a heavy one
(the 250 newest tickets of an org with relationships and aggregates, ramped from a quarter of
capacity to several times it), each at a fixed rate whatever the desk answers. A run on
2026-10-06 (2 reps, medians, other applications open on the machine; the load at its start is
in the manifest) found the heavy request's capacity alone to be **ash-rust 703/s, Ash 265/s**.
Offering both the same heavy rate:

| Heavy offered | Rust cheap p50 / p99 ms | Rust heavy answered/s | Elixir cheap p50 / p99 ms | Elixir cheap errors | Elixir heavy answered/s |
|---|---|---:|---|---:|---:|
| none | 1 / 7.2 | - | 1.5 / 4.5 | 0 | - |
| 133/s (0.5× Ash's capacity) | 1.1 / 9.8 | 133 | 1.4 / 5.1 | 0 | 133 |
| 265/s (1×) | 0.9 / 8.9 | 265 | 54 / 168 | 63 | 245 |
| 398/s (1.5×) | 0.8 / 5.4 | 398 | 144 / 800 | 910 | 211 |
| 531/s (2×) | 0.9 / 16 | 531 | 123 / 2,069 | 1,399 | 174 |
| 796/s (3×) | 498 / 826 | 654 | 108 / 1,554 | 2,006 | 113 |
| 1,062/s (4×) | 1,310 / 1,621 | 631 | 109 / 1,763 | 2,447 | 75 |

Each window has 5,000 cheap requests; ash-rust answered all of them without an error at every
step. Memory: ash-rust's server stayed under 150 MiB up to 2× and reached 6 GB at 4× as its
queue grew; Ash's went from 0.4 GB to 2.3 GB at 1× and 9.7 GB at 4×. After the heavy stream
stopped at 4×, ash-rust's cheap p99 was back within twice its baseline in 0 to 4 s; Ash's was
not back within 15 s. Offered multiples of each desk's own capacity, the same shape appears at
each one's capacity: ash-rust at 125% of its own had the cheap stream at p50 0.76 s, answering
645 heavy requests a second; Ash at 300% of its own answered 119 and failed 39% of the cheap
ones.

**Read this as a baseline.** The limit wasn't the pool or Postgres: doubling the pool changed
nothing, and Postgres had 1.5 to 8 connections busy and used about 3 of 16 cores. Each server used
8–10 cores, which with Postgres and the driver is consistent with the laptop as a whole being
full. The difference between the desks is how each treats a full pool: ash-rust's has no wait
limit, Ash's sheds.

#### Pool policy

Adding a wait limit to ash-rust's pool (`ash_postgres::PoolSettings::wait_timeout`, off by
default) and letting Ash's queue instead of shed, at 800 heavy requests a second for 30 s
(one rep each):

| Configuration | Cheap p99 | Cheap failed | Heavy answered/s | Peak memory |
|---|---:|---:|---:|---:|
| ash-rust, queues (default) | 1.7 s | 0% | 632 | 6.5 GiB |
| ash-rust, 100 ms wait limit | 116 ms | 6% | 628 | 0.83 GiB |
| Ash, sheds (default) | 0.9 s | 33% | 117 | 7.4 GiB |
| Ash, `queue_target` 60 s | 6.1 s | 0% | 86 | 22.7 GiB |

A wait limit keeps ash-rust's cheap request quick (p99 116 ms against 1.7 s) at the same heavy
throughput and about an eighth of the memory, at the cost of the 6% of cheap requests that fail. When
both shed at about 100 ms, ash-rust answers about five times as many heavy requests. Making Ash
queue is worse than its default, not like ash-rust's queue: 6 s cheap p99 and 23 GiB. Given the
*same* wait limit (Rust's `wait_timeout`; Ecto's `queue_target` at half, since it drops at twice it), the
cheap request's median is the limit in both, but ash-rust's p99 stays within 20 ms of it while
Ash's is 2 to 8 times it, heavy requests take about three times the limit in both, and ash-rust
answers about 600 heavy requests a second at 800/s to Ash's 62 to 110. At 400/s, below ash-rust's
capacity and above Ash's, ash-rust answers everything and Ash fails 13–15% of the cheap requests
and about half the heavy. See the supportdesk README for the matrix and its caveats.

![Pool matrix](../examples/supportdesk/bench/results/saturation/2026-10-06-pool-matrix/chart.svg)

![Equal wait limit](../examples/supportdesk/bench/results/saturation/2026-10-06-equal-limit/chart.svg)

#### CPU-bound saturation (in memory, no database)

`--target astro` runs the same ramp on the in-memory astro-helpdesk twins. The cheap request is
`{ __typename }`; the heavy one filters, sorts and counts 5,000 tickets and answers a page of
25, so it costs CPU and nothing else. Heavy capacity alone: **ash-rust 2,141/s, Ash 456/s**.
Offering both the same heavy rate (2 reps, medians):

| Heavy offered | Rust cheap p50 / p99 ms | Rust heavy answered/s | Elixir cheap p50 / p99 ms | Elixir heavy answered/s |
|---|---|---:|---|---:|
| none | 0.9 / 3.7 | - | 0.8 / 3.8 | - |
| 456/s (1× Ash's capacity) | 0.8 / 3.8 | 456 | 5.9 / 32 | 391 |
| 911/s (2×) | 0.9 / 3.3 | 911 | 8 / 39 | 379 |
| 1,822/s (4×) | 1.2 / 56 | 1,820 | 8.1 / 41 | 387 |
| 3,644/s (8×) | 201 / 605 | 2,006 | 8.1 / 40 | 385 |

Nothing failed on either desk. Ash's cheap request settles at about 8 ms whatever it is
offered, because the BEAM gives it a slice however many heavy requests are runnable. ash-rust's
is faster until it saturates (p99 56 ms at 85% of its capacity), then waits behind the heavy
requests queued ahead of it (Tokio runs a task until it yields): about 200 ms, bounded here only
by the driver's limit of 500 heavy requests in flight, which take about 230 ms to clear at its
capacity. Ash pays in memory (up to 7 GB against under 150 MiB) and in recovery time (0–14 s
against 0–4 s). The same shape holds in each desk's own multiples of
capacity, and the supportdesk README has the detail.

![Mixed saturation, CPU-bound and in memory](../examples/supportdesk/bench/results/saturation/2026-10-06-astro-common/chart.svg)

The reports, manifests and every window are in
[`examples/supportdesk/bench/results/saturation`](../examples/supportdesk/bench/results/saturation),
each drawn as an interactive `chart.html` and a `chart.svg`:

![Mixed saturation over Postgres](../examples/supportdesk/bench/results/saturation/2026-10-06-common/chart.svg)

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
- **Elixir Implementation**: `Helpdesk.Support.open_ticket!("Printer is broken")`
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
- **Elixir Implementation**: `Ash.Query.filter(Helpdesk.Support.Ticket, status == "open") |> Ash.read!()`
- **Rust Implementation**: `Ticket::query(&customer).filter(t::status.eq("open")).all().await.unwrap()`

#### Workload 4: `load_aggregates`
- **Pipeline**:
  1. Runs query loading 3 concurrent aggregates: `ticket_count` (`count`), `open_ticket_count` (`count` with filter), and `has_tickets` (`exists`).

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
    │   ├── bench/              # End-to-end driver (bench.ts), saturation driver (saturation.ts), scenarios
    │   │   └── results/        # Saturation reports and raw windows
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

```bash
node bench/saturation.ts --fixture /tmp/fixture.json                # mixed saturation, about 12 minutes
node bench/saturation.ts --fixture /tmp/fixture.json --basis own    # a ramp in each desk's own multiples
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
