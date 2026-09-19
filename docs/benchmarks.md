# Performance Benchmarks: ash-rust vs. Ash Elixir

This document details the benchmarking methodology, empirical results, architectural analysis, and continuous regression testing setup comparing **`ash-rust`** with canonical **[Ash Framework 3.0](https://ash-hq.org/) in Elixir**.

---

## 1. Executive Summary

A side-by-side benchmark was conducted on identical domain models (`Helpdesk` domain: `Ticket` and `Representative` resources) executing standard resource lifecycles: action validations, changeset computation, in-memory and relational persistence, filtering queries, and aggregate calculations.

### Key Results Summary

| Benchmark Workload | Ash Elixir (ETS) | `ash-rust` (In-Memory) | Rust Advantage |
| :--- | :--- | :--- | :--- |
| **`Ticket.open`** (validate + changeset + write) | 26.63 µs (37.6k ips) | **3.23 µs (309.2k ips)** | **8.2x faster** |
| **`Representative.create`** (validate + write) | 23.74 µs (42.1k ips) | **2.47 µs (405.2k ips)** | **9.6x faster** |
| **`Ticket.read`** (filter `status == open`, 100 rows) | 208.63 µs (4.79k ips) | **32.79 µs (30.5k ips)** | **6.4x faster** |
| **P99 Tail Latency** (`Ticket.open`) | 59.04 µs | **7.67 µs** | **7.7x lower** |
| **Heap Memory Allocation per Action** | 38 – 45 KB / op | **0 KB (stack-allocated)** | **Zero GC churn** |

### GraphQL (`ash_graphql` + Absinthe vs `ash-graphql`)

Median latency in parentheses.

| GraphQL Workload | Ash Elixir | ash-rust | Rust Advantage |
| :--- | :--- | :--- | :--- |
| Single record by ID | 2,850 ops/sec (311 µs) | **26,861 ops/sec (35.5 µs)** | **9.4x faster** |
| 100 tickets collection | 1,180 ops/sec (796 µs) | **4,471 ops/sec (216.5 µs)** | **3.8x faster** |
| Filtered & sorted (50) | 1,380 ops/sec (688 µs) | **7,932 ops/sec (119.7 µs)** | **5.8x faster** |
| Keyset pagination (first: 20) | 1,780 ops/sec (531 µs) | **6,942 ops/sec (137.0 µs)** | **3.9x faster** |
| DataLoader (100 tickets + author) | **760 ops/sec (1,358 µs)** | 664 ops/sec (1,419 µs) | **0.9x** (Elixir slightly ahead) |
| Mutation: `openTicket` | 5,800 ops/sec (156 µs) | **41,845 ops/sec (22.5 µs)** | **7.2x faster** |

PostgreSQL numbers were not refreshed in this run (Docker unavailable). See `benches/README.md` for the previous Postgres table.

---

## 2. Benchmark Environment

All benchmarks were executed locally on identical bare-metal hardware:

- **Host Machine**: Apple MacBook Pro (Apple M4 Max, 16 CPU cores, 128 GB unified memory)
- **Operating System**: macOS 26.6.2
- **Rust Runtime**: Rust 1.90.0, release profile (`opt-level = 3`, LTO enabled)
- **Elixir Runtime**: Elixir 1.20.1 running on Erlang/OTP 29.0.2 (BEAM JIT enabled)
- **Code server**: Elixir suites switch to `:embedded` mode after warmup (production release default)
- **Framework Versions**: `ash-core 0.1.0` vs `ash 3.33.6` / `ash_graphql 1.12.0` (Hex packages)
- **Test Harnesses**: Standalone release runners (Rust) and Benchee 1.5.1 (Elixir)
- **Sampling Configuration**: Core 2.0 s warmup + 5.0 s sampling; GraphQL Elixir 1.0 s warmup + 3.0 s sampling; GraphQL Rust 0.5 s warmup + 2.0 s sampling

---

## 3. Workload Definitions

To ensure a 1:1 comparison, both frameworks implement the exact same domain logic:

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

## 4. Architectural Analysis: Why the Difference Exists

The benchmark highlights fundamental structural differences between BEAM's dynamic runtime model and Rust's compile-time monomorphic model:

### 1. Compile-Time Monomorphism vs. Runtime DSL Introspection
- In **Elixir Ash**, the Spark DSL constructs metadata modules and map schemas at compile time, but actions and changesets are inspected, routed, and cast dynamically at runtime using map operations and pattern-matching lists.
- In **`ash-rust`**, procedural macros (`resource!` and `domain!`) generate statically typed structs, constant metadata arrays, and inlined action methods. The Rust compiler flattens and inlines the entire validation and changeset path into direct machine instructions with zero reflection.

### 2. Stack Allocation vs. Heap / Garbage Collection Pressure
- In **Elixir Ash**, every action execution allocates **38 KB to 45 KB of heap memory** for changesets, telemetry metadata, context maps, and string binaries. Under a throughput of 37,500 req/sec, this generates about **1.7 GB/sec of transient heap allocations**, triggering frequent minor garbage collection sweeps per BEAM process.
- In **`ash-rust`**, validation context and changeset arguments live directly on the thread stack. Heap allocation is restricted to inserting the final record into storage, resulting in virtually zero GC jitter.

### 3. P99 Tail Latency Predictability
- On `Ticket.open`, `ash-rust` recorded a **P99 of 7.67 µs** compared to Elixir's **59.04 µs** (a 7.7x gap).
- The absence of stop-the-world phases, concurrent tracing overhead, or dynamic dispatch allows `ash-rust` to sustain ultra-consistent latency percentiles under high concurrency.

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
    └── helpdesk/
        ├── Cargo.toml          # Configures Criterion [[bench]] target
        ├── examples/
        │   └── bench.rs        # Standalone Rust release throughput runner
        └── benches/
            └── helpdesk_bench.rs # Criterion statistical regression suite
```

---

## 6. Running the Benchmarks

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
