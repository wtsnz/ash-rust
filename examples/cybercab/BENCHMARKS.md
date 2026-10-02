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

## After fix 1: nothing drops silently

A subscriber that falls behind now gets a `MISSED_EVENTS` error with the count, and the
generated client resubscribes and re-reads.

| Measure | Before | After |
| --- | ---: | ---: |
| 1,000 cabs × 100 subscribers: subscriptions ended | 16 | 14 |
| …of which ended without being told | 16 | **0** |
| Browser console at 1,000 cabs: WebSocket messages/s | 26 | **181** |
| Browser console at 1,000 cabs: moving cabs on screen | 14 (frozen) | **136** |

The next limit shows up clearly now. At 1,000 cabs, the browser's subscriptions fall
behind about 30 times a minute (27 to 36 across runs). Each time they catch up with a
re-read, so the screen is correct, but they spend part of their time re-reading rather
than streaming, and the message rate swings between runs (29 to 181/s) depending on
where they are in that cycle. Fixes 3 and 4 below are aimed at it.

## After fix 2: `ash-memory` at Ash ETS parity

- Updates check only the identities whose fields they change.
- Calculations and aggregates are computed after filter, sort and paging.
- Reads share the store.

| Fleet | `Cab.report` before | after | Seed before | after | Tick p95 before | after |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 37 µs | **20 µs** | 0.2 s | 0.2 s | 94 ms | 92 ms |
| 1,000 | 96 µs | **18 µs** | 2.2 s | **1.0 s** | 1,765 ms | 1,483 ms |
| 2,500 | 225 µs | **18 µs** | 2.5 s | **1.3 s** | 13,961 ms | 11,257 ms |

Writes no longer grow with the table. Ticks barely improve, because they're dominated by
fetching each cab by id, which copies the whole table. That's how Ash's ETS layer reads
too, so it's the limit of an ETS-style store. The scale story moves to Postgres.

Faster writes expose the fan-out wall. At 1,000 cabs and 100 subscribers, a round now
publishes in 30 ms instead of 129 ms, and 99 of the 100 subscribers fall behind (all of
them told; none silently). The benchmark's subscribers don't resubscribe, so delivery
drops to 1.4%. Fix 4 is aimed at this.

## After writing only changes: no more refetching

An update used to send the data layer the caller's whole copy of the record. One made
from a stale copy wrote that copy's other fields back over newer values: a telemetry
report could undo an operator's recall. As in Ash, an update now writes only the
attributes it changes, so the simulation reports from the copy of each cab it read at
the start of the tick, rather than fetching every cab again (a whole-table copy each).

| Fleet | Tick p50 | Tick p95 before | Tick p95 after | Actions/s before | after |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 250 | 7 ms | 92 ms | **24 ms** | 3,045 | **10,584** |
| 1,000 | 97 ms | 1,483 ms | **194 ms** | 925 | **4,771** |
| 2,500 | 279 ms | 11,257 ms | **451 ms** | 282 | **3,597** |
| 5,000 | 700 ms | skipped | 1,108 ms | – | 2,478 |
| 10,000 | 2,222 ms | skipped | 3,106 ms | – | 1,475 |

On the in-memory store, the simulation now keeps up in real time to about 4,000 cabs, up
from about 500. Past that, the transitions that still fetch a cab or trip by id (arrival,
boarding, drop-off), and the pulse's whole-fleet reads, copy whole tables.

## On Postgres

`--postgres <url>` runs the same benchmark on `ash-postgres`. These runs used Postgres 16
in Docker Desktop on the same machine. Every round trip crosses Docker's VM network,
and with `synchronous_commit` on, every commit waits for Docker's virtualized disk:
a bare `INSERT` from `psql` takes 2.3 ms that way, and 0.02 ms with it off. The two runs
below separate what the stack costs from what each durable commit costs here.

**Durable commits** (`synchronous_commit = on`, the default):

| Fleet | Seed | `Cab::get` | Find by call sign | `Cab.report` | Tick p50 | Tick p95 | Actions/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 34 | 1.5 s | 164 µs | 0.20 ms | 1,627 µs | 14 ms | 56 ms | 464 |
| 250 | 8.2 s | 308 µs | 0.70 ms | 1,571 µs | 80 ms | 384 ms | 1,003 |
| 1,000 | 29.6 s | 216 µs | 0.25 ms | 948 µs | 306 ms | **1,115 ms** | 1,173 |
| 2,500 | 36.3 s | 186 µs | 0.31 ms | 1,037 µs | 753 ms | **2,388 ms** | 879 |

**The stack itself** (`synchronous_commit = off`, the URL's
`?options=-c%20synchronous_commit%3Doff`):

| Fleet | Seed | `Cab::get` | Find by call sign | `Cab.report` | Tick p50 | Tick p95 | Actions/s |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 34 | 0.4 s | 260 µs | 0.57 ms | 303 µs | 3 ms | 9 ms | 2,265 |
| 250 | 1.5 s | 199 µs | 0.36 ms | 221 µs | 20 ms | 68 ms | 3,873 |
| 1,000 | 5.9 s | 183 µs | 0.41 ms | 222 µs | 72 ms | 248 ms | 4,081 |
| 2,500 | 8.9 s | 190 µs | 0.40 ms | 202 µs | 187 ms | 627 ms | 3,119 |
| 5,000 | 9.7 s | 176 µs | 0.39 ms | 210 µs | 362 ms | **1,201 ms** | 3,309 |
| 10,000 | 13.9 s | 166 µs | 0.46 ms | 233 µs | 694 ms | **2,437 ms** | 3,309 |

Fan-out on Postgres, at 1,000 cabs with asynchronous commit: every update reached all
100 subscribers (500,000 deliveries) at 1.0 ms p50 and 1.5 ms p99. No subscriber fell
behind, because each round takes 583 ms to publish.

- **Lookups no longer grow with the table.** Fetching a cab, or finding one by call sign,
  costs the same at 10,000 cabs as at 34, through Postgres's indexes. Only reading every
  cab grows with the fleet, as it must.
- **The stack holds about 3,300 to 4,100 actions a second**, at every fleet size. Each
  action is a full round trip: validations, the policy check, the write, `RETURNING`, the
  notification. About 200 µs each, here.
- **A tick is now one round trip after another.** At 10,000 cabs a tick makes about
  3,300 of them in sequence. With durable commits, each also waits for its own commit.
  Batching a tick's position reports into one bulk update would turn about 3,300 round
  trips and commits into a handful. That's fix 3.

## After fix 3: a tick's telemetry written together

Each tick, the simulation now sends every cab's position report in one `bulk_update`,
and every telemetry sample in one `bulk_create`. That used to be one action and one
commit per cab. On Postgres, the reports become one `UPDATE … FROM (VALUES …)` per set of
changed columns, and the samples one multi-row `INSERT`.

Tick p50 / p95 in milliseconds, before and after:

| Fleet | Postgres, durable commits | Postgres, asynchronous commit | Memory |
| ---: | ---: | ---: | ---: |
| 1,000 | 306 / 1,115 → **81 / 169** | 72 / 248 → **36 / 91** | 97 / 194 → 96 / 175 |
| 2,500 | 753 / 2,388 → **216 / 579** | 187 / 627 → **104 / 221** | 279 / 451 → 303 / 448 |
| 5,000 | skipped → **398 / 627** | 362 / 1,201 → **175 / 411** | 700 / 1,108 → 632 / 975 |
| 10,000 | skipped → **640 / 1,124** | 694 / 2,437 → **552 / 1,059** | 2,222 / 3,106 → 2,215 / 3,003 |

**On Postgres, 10,000 cabs now run in real time at p50, even with a durable commit for
every write on Docker's slow disk.** Memory barely changes, because its writes were
already cheap: its ticks are spent in the whole-table copies of an ETS-style store.

Fan-out on Postgres at 1,000 cabs and 100 subscribers stays complete (all 500,000
deliveries, 3.6 ms p99 with durable commits). In memory, where a round publishes in
30 ms, 93 of the 100 subscribers fall behind (all told, none silently). That's the next
limit: fix 4.

## After fix 4: each event resolved once for all its subscribers

A profile of fan-out at 1,000 cabs and 100 subscribers (with a buffer deep enough that
none fell behind) found where delivery went:

| Where | Share |
| --- | ---: |
| async-graphql resolving each subscriber's selection set | ~23% |
| Receiving from the broadcast channel, mostly cloning the whole notification per receiver | ~23% |
| Allocation (`malloc`), mostly those clones | ~20% (self time) |
| Writing frames to sockets | ~8% |

Two changes:
- **The pubsub shares one notification across its subscribers** (`Arc<Notification>`),
  and buffers 8,192 events per topic by default instead of 256, since a slot now holds a
  reference rather than a copy.
- **`graphql_router` shares subscriptions, as Absinthe does for AshGraphql.** Subscribers
  to the same subscription share one execution: each event is resolved and serialized
  once, and every subscriber gets the same text.

| 100 subscribers | Delivered | Latency p50 | p99 |
| --- | ---: | ---: | ---: |
| 1,000 cabs, before (256-event buffer) | 1.4% (99 fell behind) | – | – |
| 1,000 cabs, before (deep buffer) | 100% | ~50 ms | ~120 ms |
| 1,000 cabs, after | **100%** | **29 ms** | **67 ms** |
| 5,000 cabs, after | **100%** (2.5 million deliveries) | 117 ms | 271 ms |

`--pubsub-capacity` sets the benchmark's buffer, to separate what delivery costs from what
a shallow buffer drops.

## The browser, under real load

With fan-out complete, the browser gets the whole stream. Measured as before, on the
in-memory store:

| Fleet | Mode | FPS | Frame p95 | Main thread blocked | WS msgs/s | Missed/min | Re-reads | Re-read traffic |
| ---: | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1,000 | console | 59 | 16.8 ms | 0 | 640 | 0 | – | – |
| 1,000 | wall | 60 | 16.8 ms | 0 | 439 | 0 | – | – |
| 4,000 | console | **4** | **383 ms** | **707 ms/s** | 897 | 3 | 3.6/s | 3.2 MB/s |
| 4,000 | wall | 55 | 33 ms | 0 | 519 | 0 | 15/s | **13 MB/s** |

At 1,000 cabs the console takes 640 messages a second (up from 26 at the start) at a
steady 60 fps. At 4,000 the browser is the limit, and it isn't the map: wall mode draws
1,753 moving cabs at 55 fps. Three things are left, all in the front end:

- **React work.** Every live update sets React state on its own, about 900 times a
  second. The console's trip feed renders every active trip (about 800 rows) on each
  update. The main thread is blocked 70% of the time. A browser profile hasn't
  confirmed which of these dominates.
- **Each live update copies its list.** Patching a list finds and replaces the record in
  a new array, at 900 updates a second over a list of 4,000.
- **Filtered lists re-read in full.** A filtered, sorted or limited live query re-reads
  itself after changes. At 4,000 cabs that's 13 MB/s in wall mode.

## After fix 5: the browser at 4,000 cabs

A CPU profile of the console (`frontend/scripts/profile-browser.mjs`) put the time
somewhere other than expected. React was a few percent. **46% of the main thread was
MapLibre's `findMatches`**, matching symbols across tiles during placement. Every
animation frame, the map rebuilt the GeoJSON for all 4,000 cabs so they would glide
between reports, and MapLibre re-tiled and re-placed every one of them, 60 times a
second.

- **The fleet is a WebGL layer** (`frontend/src/lib/cabLayer.ts`). Each cab's previous and
  new positions go to the GPU once, when a report arrives, and the shader glides it
  between them. A frame is a draw call: no GeoJSON, tiling or symbol placement.
  - Clicks and hovers pick the nearest cab on screen.
  - The selected cab's call sign is a DOM marker.
  - Updates only mark what changed. The frame loop hands cabs over at most once a frame,
    and redraws trails, zones, hubs, alerts and the journey at most four times a second.
- **The trip feed shows 60 trips** and counts the rest, instead of rendering every trip on
  the road (2,224 rows).
- **Live queries notify once a frame and place changes themselves** (`ash-typescript`):
  - However fast changes arrive, a listener hears its list at most once an animation
    frame, and records are found through an index rather than a search.
  - When the client can evaluate a query's filter and sort exactly as the server does, a
    change goes straight into, out of or along the list, with no re-read.

| 4,000 cabs | FPS | Frame p95 | Main thread blocked | Heap | Re-read traffic |
| --- | ---: | ---: | ---: | ---: | ---: |
| Console, before | 4 | 383 ms | 707 ms/s | 393 MB | 3.2 MB/s |
| Console, after | **59** | **16.8 ms** | **0** | **77 MB** | **0.7 MB/s** |
| Wall, before | 55 | 33 ms | 0 | 204 MB | 13 MB/s |
| Wall, after | **59** | **16.8 ms** | 0 | 136 MB | **0.9 MB/s** |

Both take 700 or so live messages a second with no missed events. The re-reads that
remain are the small paged lists (the 50 most recent trips, the pulse history), which
only the server can fill when a record leaves them.

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
