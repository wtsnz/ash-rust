# How far the command center scales

A baseline measured before any optimization, to find out what limits the stack and fix it
at the root. Every number below comes from the two harnesses in this example. Rerun them
after a change and compare.

```bash
# Server: store operations, simulation ticks, and fan-out over /graphql/ws
cargo bench -p cybercab --bench scale
cargo bench -p cybercab --bench scale -- --fleets 34,1000 --subscribers 1,10 --json after.json

# Browser: start the API with a fleet, serve a production build against it, then measure
FLEET=1000 PORT=47400 cargo run -p cybercab --release
cd frontend && PUBLIC_API_URL=http://127.0.0.1:47400 bun run build
HOST=127.0.0.1 PORT=4392 node dist/server/entry.mjs
node scripts/bench-browser.mjs --url http://127.0.0.1:4392 [--wall]
```

The baseline was taken on an Apple M4 Max (16 threads), in release mode, on the in-memory
data layer. `benches/baseline.json` holds the raw numbers.

## The simulation

One tick is one second of a live fleet, so a tick has to finish in under 1000 ms.

| Fleet | Seed | `Cab::get` | Read all cabs | Find by call sign | `Cab.report` | Record a sample | Tick p50 | Tick p95 | Actions/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 34 | 0.02 s | 29 µs | 0.08 ms | 0.03 ms | 21 µs | 7 µs | 1.1 ms | 3.7 ms | 7,483 |
| 250 | 0.2 s | 298 µs | 0.64 ms | 0.36 ms | 37 µs | 7 µs | 24 ms | 94 ms | 3,060 |
| 1,000 | 2.2 s | 1,327 µs | 2.7 ms | 1.3 ms | 96 µs | 7 µs | 434 ms | **1,765 ms** | 824 |
| 2,500 | 2.5 s | 4,090 µs | 8.5 ms | 4.1 ms | 225 µs | 7 µs | **2,979 ms** | **13,961 ms** | 230 |
| 5,000 | skipped: projected at over 10 minutes | | | | | | | | |

**The limit is about 500 cabs.** Ticks miss the budget somewhere between 250 cabs
(94 ms at p95) and 1,000 (1.8 s). Past that, the fleet falls further behind real time with
every tick.

Throughput also *drops* as the fleet grows, from 7,483 actions a second at 34 cabs to 230
at 2,500, so the cost of each action grows with the size of the data. Recording a
telemetry sample stays flat at 7 µs, which shows the action pipeline itself is cheap.
The cost is in how the store reads and checks rows:

1. **Every read copies the whole table.** `Cab::get` is a query filtered on the primary
   key (`ash-core/src/engine/lifecycle.rs:16`). The in-memory store clones every row in
   the table and then filters (`ash-memory/src/lib.rs:279`), with no primary-key lookup
   and no index. Fetching one cab out of 2,500 costs 4 ms. The simulation fetches each
   moving cab every tick, so a tick's cost grows with the square of the fleet.
2. **Every write scans the table for identity conflicts.** `check_identities`
   (`ash-memory/src/lib.rs:40`) compares the row against every other row, on every create
   and every update, even when the update doesn't touch an identity's fields. A cab has
   two identities (call sign and VIN), so each position report scans the fleet twice.
   That's why `report` grows from 21 µs to 225 µs, and why seeding is quadratic.
3. **Calculations and aggregates are computed before filtering.** When a query asks for
   them, they're computed for every row in the table, not just the rows it returns.
4. **One lock covers every table**, so reads and writes to unrelated resources queue
   behind each other.

## Fan-out

Each round, every cab reports once. Each round is sent over `/graphql/ws` to every
subscriber of `cabUpdated`.

| Fleet | Subscribers | Publish a round | Delivered | Latency p50 | p99 | Subscriptions ended |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 34 | 1 | 1.2 ms | 100% | 0.1 ms | 0.3 ms | 0 |
| 34 | 10 | 1.6 ms | 100% | 0.5 ms | 1.5 ms | 0 |
| 34 | 100 | 1.3 ms | 100% | 5.4 ms | 11.7 ms | 0 |
| 1,000 | 1 | 113 ms | 100% | 0.1 ms | 0.2 ms | 0 |
| 1,000 | 10 | 119 ms | 100% | 0.2 ms | 0.3 ms | 0 |
| 1,000 | 100 | 129 ms | **87%** | 2.9 ms | 24.5 ms | **16** |
| 5,000 | 1 | 2,767 ms | 100% | 0.6 ms | 1.4 ms | 0 |
| 5,000 | 10 | 2,997 ms | 100% | 0.6 ms | 2.0 ms | 0 |
| 5,000 | 100 | 3,819 ms | 100% | 1.1 ms | 3.3 ms | 0 |

**Subscriptions that fall behind are dropped silently.** At 1,000 cabs and 100
subscribers, 16 subscriptions were ended by the server and 13% of updates never arrived.
At 5,000 cabs everything arrived, but only because publishing a round took nearly
3 seconds. The slow store throttled the publisher enough for subscribers to keep up.
Once the store is fast, this becomes the wall.

The cause is three bugs that combine:

1. **Each topic buffers 256 events** (`ash-pubsub/src/lib.rs:40`), in a Tokio
   `broadcast` channel that overwrites the oldest event when a receiver falls behind.
2. **A lagging subscription ends.** The GraphQL subscription resolvers loop on
   `while let Ok(notif) = sub.recv().await` (`ash-graphql/src/subscription.rs:59`, `109`
   and `155`). The first `Lagged` error ends the stream, and the client receives
   `complete`.
3. **The live client doesn't recover.** When the server completes a subscription, the
   generated client drops it (`ash-typescript/src/live.rs:191`). The live query doesn't
   resubscribe or re-read, so that part of the screen stops updating with no error.

The live demo shows the same thing. With 1,000 cabs, a single subscriber's `cabUpdated`
subscription was completed by the server within 30 seconds, after about 1,300 updates.
One tick's burst of reports was enough to overflow the buffer.

## The browser

Headless Chrome on the M4 Max's GPU (ANGLE Metal), over 20 seconds after a 15-second
settle.

| Fleet | Mode | FPS | Frame p95 | Long tasks | JS heap | WS messages/s | WS KB/s | Cabs moving on screen |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 34 | console | 60 | 16.7 ms | 0 | 36 MB | 9.7 | 6.7 | 6 |
| 34 | wall | 60 | 16.7 ms | 0 | 66 MB | 16.8 | 11.1 | 14 |
| 1,000 | console | 60 | 16.8 ms | 0 | 112 MB | 26.2 | 32.9 | 14 |
| 1,000 | wall | 60 | 16.7 ms | 0 | 104 MB | 36.4 | 43.9 | 206 |
| 2,500 | console | 58 | 16.8 ms | 0 | 89 MB | 1.9 | 1.8 | 0 |
| 2,500 | wall | 60 | 16.8 ms | 0 | 105 MB | 1.1 | 0.6 | 95 |

The browser never struggled, but these numbers say little about its limits yet, because
it never received a real load. The server couldn't produce one (ticks were seconds long
at 2,500 cabs), and the subscriptions that did carry load were ended (14 cabs moving in
the console at 1,000, against 206 in wall mode). These need re-measuring once the server
keeps up.

## What to fix, in order

1. **The live pipeline must never drop silently.** This is a correctness bug, not just a
   speed limit: a screen can freeze with no sign anything is wrong.
   - A lagging subscriber should be told it missed events, and its live query should
     re-read, rather than the stream ending.
   - The client should resubscribe if the server does complete a subscription.
   - Ash on BEAM doesn't have this failure: Phoenix PubSub delivers into each process's
     mailbox, which doesn't drop messages.
2. **Bring `ash-memory` up to Ash ETS parity, and scale on Postgres.** `ash-memory` is
   the counterpart of `Ash.DataLayer.Ets`, which Ash documents as "for testing and
   lightweight usage". Ash ETS also copies the whole table on every read, primary-key
   lookups included. But `ash-memory` is slower than ETS in three places:
   - It checks identities on every update. Ash only pre-checks an identity on create or
     when one of its fields changes (`Ash.Changeset`).
   - It computes aggregates and calculations before filtering. Ash computes them after
     filter, sort and limit.
   - One lock covers every table. ETS has a table per resource, with read concurrency.

   Past that, the scale story belongs on a production data layer: the Cybercab gets a
   Postgres mode, and this benchmark runs against it.
3. **Batch the heartbeat.** Each moving cab reports through its own action every tick.
   One bulk update per tick (Ash's `bulk_update`), with its notifications delivered as
   one batch, would cut both the pipeline cost and the WebSocket messages.
4. **Encode an event once for all its subscribers.** Each subscriber currently resolves
   and serializes every event itself. That's cheap at 10 subscribers (0.3 ms p99) and
   costs 24 ms at p99 with 100.
5. **Measure the browser again.** Once the server sustains thousands of moving cabs, the
   next limit is likely MapLibre re-uploading every cab's position on each change. A
   WebGL layer fed batched positions is the usual answer.
