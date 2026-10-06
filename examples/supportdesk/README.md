# Supportdesk

A multi-tenant support desk, built twice, here on ash-rust and in
[`examples/elixir/supportdesk`](../elixir/supportdesk) on Ash with AshPostgres,
AshGraphql and AshTypescript, to benchmark real-world Ash features against each other: policies and field
policies under an actor, multitenancy, calculations and aggregates in filters and sorts,
relationship loading, keyset pages with counts, validations and changes, managed
relationships, a state machine, atomic updates, optimistic locking, a generic action in a
transaction, bulk actions, and subscriptions.

Both apps implement the contract below. A request that one answers, the other answers the
same way: [`parity`](#parity) checks it.

## Resources

Every resource but `Org` belongs to one, by attribute multitenancy on `org`: the tenant
is the org's slug, and reads, writes and identities are scoped to it.

| Resource | Attributes | Relationships, aggregates, calculations |
|---|---|---|
| `Org` | `id`, `name`, `slug` (unique) | |
| `Agent` | `id`, `org`, `name`, `email` (unique in the org), `role` (`admin`, `agent`, `viewer`), `active` | `assigned_tickets`; `open_assigned` (count of open ones) |
| `Tag` | `id`, `org`, `name` (unique in the org) | |
| `Ticket` | `id`, `org`, `subject`, `body`, `status`, `priority` (1–4), `confidential`, `requester_email`, `assignee_id`, `author_id`, `view_count`, `reopen_count`, `version`, `inserted_at`, `updated_at` | `assignee`, `author` (agents); `comments`; `tags` (through `TicketTag`); `comment_count`, `public_comment_count`, `has_internal_notes`; `weight = priority * 10`, `subject_length` |
| `Comment` | `id`, `org`, `ticket_id`, `author_id`, `body`, `internal`, `inserted_at`, `updated_at` | `ticket`, `author` |
| `TicketTag` | `id`, `org`, `ticket_id`, `tag_id` | |
| `AuditEvent` | `id`, `org`, `ticket_id`, `actor_id`, `kind`, `inserted_at`, `updated_at` | |

### Ticket actions

| Action | Kind | What it does |
|---|---|---|
| `read` | read | primary; keyset pages |
| `open` | create | accepts `subject`, `body`, `priority`, `confidential`, `requester_email`; `comments` argument, created with the ticket (managed relationship); the actor becomes the author; requires `requester_email` and validates the subject's length (3–200) and the priority (1–4) |
| `assign` | update | accepts `assignee_id` |
| `start`, `hold`, `resolve`, `reopen`, `close` | update | the status machine: `new` → `open` → `pending`/`resolved` → `closed`, `resolved` → `open`; `reopen` also adds one to `reopen_count`, atomically |
| `view` | update | adds one to `view_count`, atomically |
| `edit` | update | accepts `subject`, `priority`, under the `version` optimistic lock (as is every update) |
| `route` | generic | in one transaction: opens a ticket with its comments, assigns it to the active agent or admin with the fewest open tickets (then by name), and records an `AuditEvent`; returns the ticket's id |
| `destroy` | destroy | primary |

`Comment` has `read` and `create`, `AuditEvent` has `read` and `record`, and `Org`,
`Agent`, `Tag` and `TicketTag` have `read`. Every resource but `AuditEvent` also has a
`seed` create that accepts every attribute, for loading the fixture.

### Policies

The actor is an agent: its `id` and `role`. An `admin` may do anything in its org.

- **Tickets:** an `agent` reads every ticket and writes them; a `viewer` reads only tickets
  that aren't `confidential`, and writes none.
- **Comments:** an `agent` reads every comment; a `viewer` reads only those that aren't
  `internal`.
- **Field policy:** a ticket's `requester_email` is visible only to an `admin` and to the
  ticket's assignee; anyone else reads `null`.

## The API

Both apps serve the same API on `PORT` (default 4000):

- `POST /graphql`: AshGraphql's schema: `getTicket`, `listTickets` (keyset pages with
  `count`, `filter`, `sort`), the other resources' `get` and `list`, and each create,
  update and destroy action as a mutation.
- `GET /graphql/ws`: subscriptions: `ticketCreated`, `ticketUpdated`, `ticketDestroyed`.
- `POST /rpc/run`: AshTypescript's RPC, the Elixir desk's `typescript_rpc` actions run by
  name: `list_tickets`, `get_ticket` (by `id`), `open_ticket`, `assign_ticket`,
  `start_ticket`, `hold_ticket`, `resolve_ticket`, `reopen_ticket`, `close_ticket`,
  `view_ticket`, `edit_ticket`, `destroy_ticket`, `route_ticket`, `list_comments`,
  `create_comment`, `list_agents`, `list_tags` and `list_audit_events`. The Elixir desk
  generates the TypeScript client for them, [`client/ash_rpc.ts`](client/ash_rpc.ts)
  (`mix ash_typescript.codegen`), which works against either desk. `POST /rpc/validate`
  validates a request's input without running it.
- `POST /api/route`, `POST /api/bulk`, `POST /api/edit`: JSON for what AshGraphql serves
  but ash-graphql doesn't yet (generic actions, managed relationship inputs), and for a
  client-held lock version: `route` runs the generic action; `bulk` creates, assigns and
  destroys `count` tickets with bulk actions; `edit` edits a ticket at the version the
  client read, failing as stale if it has changed since.
- `GET /health`.

Every request names its tenant and actor in headers: `x-org` (the org's slug), `x-actor`
(the agent's id) and `x-role` (its role). Without them a request runs with no tenant and
no actor.

## The fixture

`cargo run --release -p supportdesk --bin fixture -- --out fixture.json` writes the data
both apps load: 10 orgs, 100 agents, 200 tags, 10,000 tickets, 50,000 comments and the
tickets' tags, with fixed ids, timestamps and distributions from a seeded generator
(SplitMix64). The data is uneven on purpose: tickets without comments, tied timestamps,
unassigned and confidential tickets, internal comments.

An app started with `FIXTURE=fixture.json` and `DATABASE_URL` empties its tables and loads
the fixture through each resource's `seed` action, then serves. It reports how long that
took on `GET /health/seeded`.

## Running both

```bash
cargo run --release -p supportdesk --bin fixture -- --out /tmp/fixture.json

# ash-rust, on :4701
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/supportdesk_rust \
  FIXTURE=/tmp/fixture.json PORT=4701 cargo run --release -p supportdesk

# Ash, on :4702
cd examples/elixir/supportdesk && mix deps.get && MIX_ENV=prod mix release
DATABASE_URL=postgres://postgres:postgres@127.0.0.1:5432/supportdesk_elixir \
  FIXTURE=/tmp/fixture.json PORT=4702 _build/prod/rel/supportdesk/bin/supportdesk start
```

## Parity

```bash
cargo run --release -p supportdesk --bin parity -- \
  --rust http://127.0.0.1:4701 --elixir http://127.0.0.1:4702 --fixture /tmp/fixture.json
```

sends both apps the same requests, each as an admin, an agent and a viewer, and compares
the answers: reads (an inbox and its second page, a dashboard filtered and sorted by
aggregates and calculations, nested relationships with limits), mutations and their
failures, RPC actions and their failures (every error compared whole: its type,
message template, vars, fields, path and details), RPC validation, the JSON endpoints,
and a subscription. Both
must have just loaded the fixture. Where ash-rust still falls short of Ash, or Ash's
packages behave unexpectedly, [GAPS.md](GAPS.md) says how.

```bash
node client/smoke.ts --rust http://127.0.0.1:4701 --elixir http://127.0.0.1:4702 --fixture /tmp/fixture.json
```

does the same through the generated TypeScript client (Node 22.18 or later runs it as it
is).

## Benchmark

```bash
cargo build --release -p supportdesk --bins
(cd ../elixir/supportdesk && MIX_ENV=prod mix release --overwrite)
node bench/bench.ts --fixture /tmp/fixture.json            # about 8 minutes
node bench/bench.ts --fixture /tmp/fixture.json --quick    # one short rep of each, about 3
```

[`bench/bench.ts`](bench/bench.ts) drives both desks through the API real clients use:
RPC through the generated AshTypescript client, GraphQL, and JSON where the desks serve
nothing else. It needs Node 22.18 or later and `psql`, and nothing to install.

- **Same data.** Each desk loads the fixture once into a template database
  (`supportdesk_bench_<desk>_tpl`, analyzed), and runs on a fresh copy of it
  (`CREATE DATABASE … TEMPLATE`), on ports 4711 and 4712.
- **Same answers first.** Nothing is timed until `parity` and the smoke test pass, each
  on fresh copies, and each read scenario's first requests answer alike on both desks.
  A scenario they answer differently isn't timed, and the report says why.
- **Reads** (`inbox`, `dashboard`, `detail`) run closed loop: 16 clients, each asking
  again when answered, as admins, agents and viewers of every org, from a seeded
  generator, so both desks are asked the same things in the same order.
- **Writes** run open loop at fixed rates, so both desks do the same work and a desk
  that falls behind is charged for the wait (latency counts from when each request was
  due): `workflow`, `route`, `counters` (checked after: every acknowledged view
  counted), `edit races` (checked: exactly one of each pair wins), `bulk`, and `events`
  (`ticketUpdated` to 20 subscribers, every delivery checked). Each rep runs on a fresh
  copy.
- **Fair order.** Reps alternate which desk goes first. Reads run 3 reps per desk, writes
  2.
- **What's reported,** in operations (a read is one request; a write scenario's step may
  be several): throughput, the operations that succeeded and finished within the window;
  latency, every operation started or due within it, however late it finished (p95 from
  1,000 samples, p99 from 10,000); failures, with their latency kept apart; the server's
  CPU time over the window per 1,000 operations and its peak memory; and how busy the
  driver was. Each run writes `bench/runs/<time>/`: a manifest (revisions, binary
  hashes, machine, Postgres, options), every window in `results.jsonl`, the desks' logs,
  and `report.md`, which gives each scenario's medians and ratio and flags a difference
  within noise (5%, or the spread between reps). There's no overall figure.

Postgres, both desks and the driver share one machine, so the figures compare the desks
with each other rather than measure capacity. [GAPS.md](GAPS.md) notes where a scenario
steps around a difference between the desks.

### Latest results

A full run on 2026-10-04: ash-rust against Ash (Elixir), on an
Apple M4 Max (16 cores) with PostgreSQL 16.15. Parity (115 checks) and the smoke test (7)
matched first. Reads ran 3 reps of 5 s per desk, writes 2 reps per desk, each figure the
median over reps. **Ratio** is how many times better ash-rust does: throughput for
reads, p50 latency for writes.

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

Every check held: each acknowledged view counted, exactly one edit of each race won, and
every update delivered once to every subscriber. The only failures were expected ones:
`counters` views that lost a race under the ticket's optimistic lock, answered
`not_found` (19 on the Rust desk, 9 on the Elixir desk, across both transports and
reps), and one `events` request on the Elixir desk. No request was sent late or left
unanswered. `detail` over GraphQL varied most between reps on both desks (Rust 7476 to
about 4700/s, Elixir about 2000 to 1212/s), so read that row as the noisiest.

## Mixed saturation

`bench/saturation.ts` asks what a desk does to a small request while large ones fill it.
Two streams arrive at once, each at a fixed rate whatever the desk answers (open loop):

- **cheap**: one ticket by id over GraphQL, as a viewer, 500 a second;
- **heavy**: the 250 newest tickets of an org with their assignee, author, tags,
  aggregates and five comments each, as its admin, ramped in 10 s steps from a quarter of
  the desks' capacity to several times it.

```bash
node bench/saturation.ts --fixture /tmp/fixture.json                 # about 12 minutes
node bench/saturation.ts --fixture /tmp/fixture.json --basis own     # a ramp in each desk's own multiples
node bench/saturation.ts --fixture /tmp/fixture.json --quick         # one short rep
```

It needs what `bench.ts` does, and takes `--levels`, `--cheap-rate`, `--timeout` and
`--max-in-flight`. It first measures each desk's capacity for the heavy request alone
(closed loop, 24 clients), then offers both desks, on a fresh copy of the data, either the
same heavy rates, multiples of the slower desk's capacity (`--basis common`, the default),
or multiples of their own (`--basis own`). After the last step the heavy stream stops and
the cheap one is watched until its p99 is back within twice its baseline. Latency counts
from when a request was due; one unanswered after 10 s is given up on, as a client would.
The driver holds at most 3,000 heavy requests unanswered, so a request due past that is
reported as *not sent*: the driver's limit, never a desk's refusal, and a bound on how
long a desk that queues can queue.

### Latest results

The reports, manifests and every window of two runs are in
[`bench/results/saturation`](bench/results/saturation). A run on 2026-10-06 (2 reps; medians; Apple M4 Max, 16 cores, with other applications
open, load average 9–17 at the start), against the Elixir desk's `mix release`. Heavy
capacity alone: **ash-rust 703/s, Ash 265/s**. Both desks were offered the same heavy rate:

| Heavy offered | Rust cheap p50 / p99 ms | Rust heavy answered/s | Elixir cheap p50 / p99 ms | Elixir cheap errors | Elixir heavy answered/s |
|---|---|---:|---|---:|---:|
| none | 1 / 7.2 | - | 1.5 / 4.5 | 0 | - |
| 66/s (0.25×) | 1.1 / 8.4 | 66 | 1.3 / 5 | 0 | 66 |
| 133/s (0.5×) | 1.1 / 9.8 | 133 | 1.4 / 5.1 | 0 | 133 |
| 265/s (1×) | 0.9 / 8.9 | 265 | 54 / 168 | 63 | 245 |
| 398/s (1.5×) | 0.8 / 5.4 | 398 | 144 / 800 | 910 | 211 |
| 531/s (2×) | 0.9 / 16 | 531 | 123 / 2,069 | 1,399 | 174 |
| 796/s (3×) | 498 / 826 | 654 | 108 / 1,554 | 2,006 | 113 |
| 1,062/s (4×) | 1,310 / 1,621 | 631 | 109 / 1,763 | 2,447 | 75 |

Of the 5,000 cheap requests in each window, Rust answered every one without an error, at every step, though slowly past its capacity. Elixir's
errors are 1% of them at 1×, 28% at 2× and 49% at 4×. Rust never refused a request;
its 4× step had 2,211 heavy requests the driver held back.

In each desk's own multiples (`--basis own`), the same shape appears at their own
capacities: Rust, at 1× (697/s), had the cheap stream at p50 106 ms and p99 169 ms; at 1.25×
(872/s) p50 756 ms, with the heavy request's p50 at 2.5 s, answering 645/s; at 3× it
answered 675/s. Ash at 1× (266/s) had cheap p50 55 ms, p99 115 ms and 64 errors; at 3×
(798/s) 119 heavy requests answered a second and 39% of the cheap requests failing.

![Mixed saturation over Postgres: cheap and heavy request latency, answered and failed, memory, CPU and recovery, ash-rust and Ash](bench/results/saturation/2026-10-06-common/chart.svg)

Every run is also drawn as `chart.html` (open it in a browser: hover for every value, a table
view, light and dark) and `chart.svg` (the image above) beside its report; `bench/chart.ts`
redraws them from a run's `results.jsonl`, and `saturation.ts` runs it at the end of a run.

What this shows:

- **Below its capacity, ash-rust keeps the cheap request quick whatever else it's doing**
  (p99 under 20 ms up to 90% of its capacity), and Ash does up to about half of its own:
  its cheap p99 was 5 ms at 50% of its capacity, 64 ms at 90% and 115 ms at 100%.
- **Past capacity the two fail differently.** Rust queues: nothing is refused, memory grows
  with the queue (6 GB resident at 4×, from under 150 MB at 2×), throughput holds at about
  650/s, and every request, the cheap ones included, waits behind it (cheap p50 0.5 s at 3×,
  1.3 s at 4×). Ash sheds: Ecto's pool drops a request that has waited about 100 ms
  (`connection not available and request was dropped from queue`), so the cheap request that
  succeeds waits a steady ~110 ms, and the rest fail; but what it answers falls as the
  load grows (heavy answered/s from 245 at 1× to 75 at 4×), and it hadn't recovered within
  15 s of the heavy stream stopping at 4×, where Rust had within 4 s.
- **Neither desk isolates the cheap request from the heavy ones.** Both draw on one
  20-connection Postgres pool and one database, and they share the machine.

What it doesn't show: whether a preemptive scheduler matters. Neither server was CPU-bound
(Rust used 8–9 of the 16 cores past capacity, Ash about 10), and the heavy request waits on
Postgres, so the limit was the pool and the database, and the difference between the desks
is how each treats a full pool: Rust's has no wait limit, Ash's sheds. The next section takes
the database out. Treat the figures as a baseline to compare a later change against, and the
machine's load, noted in each run's `manifest.json`, as the largest source of noise.

### CPU-bound saturation (no database)

```bash
node bench/saturation.ts --target astro                 # about 10 minutes
node bench/saturation.ts --target astro --basis own
```

`--target astro` runs the same ramp against the in-memory astro-helpdesk twins, in ash-rust
and in Ash (build `cargo build --release -p astro-helpdesk` and `MIX_ENV=prod mix release` in
`examples/elixir/astro-helpdesk`; no Postgres, no fixture). The cheap request is
`{ __typename }`, which costs almost nothing, so its latency is how soon a worker gets to
it. The heavy one filters, sorts and counts 5,000 tickets in memory and answers a page of
25, so what it costs a desk is CPU and nothing else. The driver holds at most 500 heavy
requests unanswered here (`--max-in-flight`), because Ash's memory grows with every request
in flight.

A run on 2026-10-06 (2 reps, medians, other applications open). Heavy capacity alone:
**ash-rust 2,141/s, Ash 456/s**, 4.7 times. Offering both the same heavy rate:

| Heavy offered | Rust cheap p50 / p99 ms | Rust heavy answered/s | Elixir cheap p50 / p99 ms | Elixir heavy answered/s |
|---|---|---:|---|---:|
| none | 0.9 / 3.7 | - | 0.8 / 3.8 | - |
| 228/s (0.5× Ash's capacity) | 1 / 4.7 | 228 | 0.7 / 2.5 | 228 |
| 456/s (1×) | 0.8 / 3.8 | 456 | 5.9 / 32 | 391 |
| 911/s (2×) | 0.9 / 3.3 | 911 | 8 / 39 | 379 |
| 1,822/s (4×) | 1.2 / 56 | 1,820 | 8.1 / 41 | 387 |
| 3,644/s (8×) | 201 / 605 | 2,006 | 8.1 / 40 | 385 |

No request failed or timed out on either desk. Cores busy: Ash 13.6 of 16 from 1×; ash-rust
1.9 at 1×, 10.5 at 4×, 13.3 at 8×. Memory: ash-rust under 150 MiB throughout; Ash up to
about 7 GB (0.4 GB idle). In each desk's own multiples, ash-rust's cheap request is p50 99 ms,
p99 272 ms at 90% of its capacity, and p50 about 190 ms, p99 460–680 ms from 125% to 300%;
Ash's is p50 4 ms, p99 17 ms at 100% of its capacity, and p50 about 8 ms, p99 37–47 ms from
125% to 300%. After the heavy stream stopped, ash-rust's cheap p99 was back within twice its
baseline in 0 to 4 s, Ash's in 0 to 14 s.

![Mixed saturation, CPU-bound and in memory: cheap and heavy request latency, answered and failed, memory, CPU and recovery, ash-rust and Ash](bench/results/saturation/2026-10-06-astro-common/chart.svg)

What this shows:

- **Once CPU is the limit, Ash keeps the cheap request quick and ash-rust doesn't.** Ash's
  cheap p50 settles at about 8 ms, p99 about 40 ms, and stays there however much heavy work
  is offered, up to 8 times its capacity. ash-rust's is flat and faster until it saturates
  (p99 under 10 ms to half its capacity, 56 ms at 85%), then jumps to about 100 ms at 90%
  and about 200 ms (p99 about 0.5 s) past it. That is the BEAM's preemption: the cheap request gets a slice however many heavy
  ones are runnable. Tokio runs a task until it yields, so a cheap task waits behind the
  heavy ones queued ahead of it. The driver's limit of 500 heavy requests in flight bounds
  that queue, and 500 requests take about 230 ms to clear at ash-rust's capacity of
  2,141/s, which is the 200 ms seen: with no limit the wait would keep growing until
  requests timed out.
- **ash-rust's saturation point is far later.** At the load that saturates Ash (456/s),
  ash-rust is at 20% of its capacity and its cheap p99 is 4 ms. It stays ahead of Ash's
  saturated tail up to about 80% of its own capacity (p99 56 ms at 85%, against Ash's 40),
  and behind it past that.
- **Ash degrades more gently**: its heavy throughput holds at 80–90% of its capacity, where
  ash-rust's holds at all of it, and neither refuses anything. What Ash pays is memory (up
  to 7 GB against 150 MiB), and up to 14 s to recover.

So the remark was right about what happens past saturation, and what it describes can be
bought in ash-rust without a preemptive scheduler: keep heavy requests from queuing ahead of
cheap ones with a limit on admission, or run them on a runtime or pool of their own.
