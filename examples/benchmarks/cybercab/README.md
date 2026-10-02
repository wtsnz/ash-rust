# Cybercab: Rust vs Elixir

The [Cybercab Command Center](../../cybercab) is built twice: on ash-rust, and on Ash
for Elixir the way an Ash developer would build it ([`examples/elixir/cybercab`](../../elixir/cybercab)).
Both serve the same GraphQL schema, AshGraphql's, so the same frontend, SDK and load
work against either. This benchmark drives both with the same load and compares them.

```bash
./run.sh                 # every scenario, about two minutes
./run.sh --reps 3        # three times; the report takes the median of each figure
./run.sh --full          # 200, 1,000 and 5,000 cabs, longer windows, more levels: far slower
./run.sh --servers rust --queries counted_page --scenarios reads
```

It writes `results/results.jsonl`, a line per measurement, and `results/RESULTS.md`, the
tables. `cybercab-bench report --out results` rewrites the tables from the measurements,
and `cybercab-bench compare --before A --after B` sets one server's figures from two runs
side by side: how much a change moved them.

A two-minute run measures each figure over a second or a few, with the fleet moving, so
figures move ±20% from run to run; a change smaller than that needs `--reps` or `--full`
to tell from noise.

## What runs

`run.sh` starts Postgres 16 in Docker (or uses `POSTGRES`), builds the Rust server and
the driver with `cargo --release`, and the Elixir server with `MIX_ENV=prod mix release`.
The driver, `cybercab-bench`, then boots each server once per fleet size, seeded and with
the simulation running, measures every scenario against it, and stops it. It talks to the
servers only over HTTP and WebSocket, as any client would.

There are three servers:

| | Heartbeat: every cab's position report, each tick |
|---|---|
| `rust` | one `bulk_update`, each cab with its own report (`Cab::bulk_update`) |
| `elixir-concurrent` | each cab's `report` update, 20 at a time (the pool's size) |
| `elixir-upsert` | one `Ash.bulk_create` upsert, which notifies as creates |

Reads and commands don't depend on how the heartbeat is written, so they run on
`rust` and `elixir-concurrent` only.

## Scenarios

| | Scenario | Setting | Measures |
|---|---|---|---|
| A | **Reads** | while the fleet moves | requests/s and latency at 1 and 16 concurrent clients (and 64 with `--full`), for: a cab by id, the whole fleet, recent trips with riders, cabs and zones (relationships), aggregates, a keyset page, and a page with its count, as the SDK's `page()` reads it |
| B | **Commands** | while the fleet moves | the same, for pulling cabs over and sending them on, and a trip's whole life through its state machine (requested to completed) |
| C | **Simulation** | 200 cabs (and 1k, 5k with `--full`) | tick p50/p95 against the one-second budget, ticks a second, CPU and memory |
| D | **Fan-out** | as C | 10 subscribers (1, 10 and 100 with `--full`) to the fleet's movements: events each hears, delivery latency, subscriptions ended |
| E | **The real room** | as C | the simulation, 20 wall displays subscribed, and 8 operators reading and commanding with a 250 ms pause: operator latency, tick health, delivery latency |
| F | **Resources** | every run | boot time (start to healthy, seeding included), CPU (cores busy) and resident memory |

Delivery latency is the time from a report's `lastSeenAt`, stamped when the simulation
builds it, to a subscriber hearing it, on the same clock. It counts only events carrying a
new report: other updates to a cab repeat its last one.

## Fairness

- Same machine, same Postgres, a database each, emptied at every start.
- Release builds on both sides, default schedulers, 20 database connections each.
- The same seed, city, routes and random generator (SplitMix64, drawn in the same order),
  so the same fleet; the simulations then diverge, since they read records in database
  order, but generate the same demand.
- The same GraphQL documents, sent to the same paths.
- A warm-up before every measurement; `--reps` for medians.

## Where the two stacks differ by design

- **Heartbeat.** Ash has no per-record bulk update, so Elixir runs one of the two
  idiomatic alternatives above.
- **Subscriptions.** ash-rust resolves each subscription document once for all its
  subscribers, and ends a subscriber that falls too far behind, saying how many events it
  missed. AshGraphql resolves through Absinthe, deduplicating identical documents, and with
  the batcher gathers notifications for up to a second under load. Phoenix buffers a slow
  subscriber without limit, which shows as memory.
- **Upsert heartbeats** reach subscribers as `cabCreated`, so the driver listens to
  `cabUpdated` and `cabCreated` on every server.
