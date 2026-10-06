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
[`bench/results/saturation`](bench/results/saturation). A run on 2026-10-06 (2 reps, medians;
Apple M4 Max, 16 cores; other applications open, load average 9–17 at the start), against the
Elixir desk's `mix release`. Heavy capacity alone: **ash-rust 703/s, Ash 265/s**.

**The same heavy load for both desks.** Each pair of rows is one load, with the same columns for
each desk, so read down a column to compare them:

| Heavy offered | | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| none | Rust | 1.0 ms | 7.2 ms | 0% | - | - | - | - | 15 MiB | 0.1 |
|  | Elixir | 1.5 ms | 4.5 ms | 0% | - | - | - | - | 416 MiB | 0.5 |
| 66/s (0.25×) | Rust | 1.1 ms | 8.4 ms | 0% | 17.8 ms | 0% | 0% | 66 | 34 MiB | 0.6 |
|  | Elixir | 1.3 ms | 5.0 ms | 0% | 40.1 ms | 0% | 0% | 66 | 526 MiB | 3.1 |
| 133/s (0.5×) | Rust | 1.1 ms | 9.8 ms | 0% | 15.7 ms | 0% | 0% | 133 | 62 MiB | 1.3 |
|  | Elixir | 1.4 ms | 5.1 ms | 0% | 41.5 ms | 0% | 0% | 133 | 577 MiB | 6.1 |
| 265/s (1×) | Rust | 0.9 ms | 8.9 ms | 0% | 15.7 ms | 0% | 0% | 265 | 92 MiB | 2.9 |
|  | Elixir | 53.6 ms | 168 ms | 1% | 279 ms | 5% | 0% | 245 | 2.2 GiB | 10.6 |
| 398/s (1.5×) | Rust | 0.8 ms | 5.4 ms | 0% | 16.1 ms | 0% | 0% | 398 | 102 MiB | 4.5 |
|  | Elixir | 143 ms | 800 ms | 18% | 488 ms | 53% | 0% | 211 | 4.9 GiB | 10.2 |
| 531/s (2×) | Rust | 0.9 ms | 16.4 ms | 0% | 16.7 ms | 0% | 0% | 531 | 140 MiB | 6.4 |
|  | Elixir | 123 ms | 2,069 ms | 28% | 468 ms | 74% | 0% | 174 | 6.1 GiB | 9.8 |
| 796/s (3×) | Rust | 498 ms | 826 ms | 0% | 1,593 ms | 0% | 0% | 654 | 3.3 GiB | 8.6 |
|  | Elixir | 108 ms | 1,554 ms | 40% | 392 ms | 89% | 0% | 113 | 7.8 GiB | 9.7 |
| 1,062/s (4×) | Rust | 1,310 ms | 1,621 ms | 0% | 3,957 ms | 0% | 21% | 631 | 6.2 GiB | 8.4 |
|  | Elixir | 109 ms | 1,763 ms | 49% | 394 ms | 90% | 5% | 75 | 9.5 GiB | 9.6 |

**Each desk at multiples of its own capacity** (`--basis own`), so each is at 100%, 125%, 200%
of what it can do:

| Heavy offered, × the desk's own capacity | | Offered | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| none | Rust | - | 1.0 ms | 4.0 ms | 0% | - | - | - | - | 16 MiB | 0.1 |
|  | Elixir | - | 1.5 ms | 6.5 ms | 0% | - | - | - | - | 414 MiB | 0.5 |
| 0.5× | Rust | 349/s | 0.8 ms | 6.5 ms | 0% | 16.2 ms | 0% | 0% | 349 | 97 MiB | 3.9 |
|  | Elixir | 133/s | 1.4 ms | 5.0 ms | 0% | 41.1 ms | 0% | 0% | 133 | 606 MiB | 5.9 |
| 0.9× | Rust | 628/s | 1.1 ms | 18.5 ms | 0% | 17.7 ms | 0% | 0% | 628 | 175 MiB | 7.8 |
|  | Elixir | 239/s | 2.4 ms | 64.2 ms | 0% | 53.9 ms | 0% | 0% | 236 | 1.4 GiB | 10.4 |
| 1× | Rust | 697/s | 106 ms | 169 ms | 0% | 337 ms | 0% | 0% | 678 | 852 MiB | 8.8 |
|  | Elixir | 266/s | 54.5 ms | 115 ms | 1% | 267 ms | 4% | 0% | 252 | 2.0 GiB | 10.7 |
| 1.25× | Rust | 872/s | 757 ms | 1,274 ms | 0% | 2,493 ms | 0% | 0% | 645 | 4.4 GiB | 8.7 |
|  | Elixir | 332/s | 126 ms | 409 ms | 11% | 496 ms | 35% | 0% | 227 | 3.6 GiB | 10.3 |
| 1.5× | Rust | 1,046/s | 1,308 ms | 1,669 ms | 0% | 3,914 ms | 0% | 20% | 639 | 5.5 GiB | 8.5 |
|  | Elixir | 399/s | 112 ms | 1,001 ms | 18% | 439 ms | 54% | 0% | 207 | 4.6 GiB | 10.0 |
| 2× | Rust | 1,394/s | 1,462 ms | 1,660 ms | 0% | 4,247 ms | 0% | 43% | 654 | 5.9 GiB | 8.3 |
|  | Elixir | 532/s | 109 ms | 1,270 ms | 26% | 412 ms | 74% | 0% | 169 | 6.0 GiB | 9.7 |
| 3× | Rust | 2,092/s | 1,434 ms | 1,693 ms | 0% | 4,262 ms | 0% | 68% | 675 | 6.6 GiB | 8.6 |
|  | Elixir | 798/s | 107 ms | 1,610 ms | 39% | 392 ms | 88% | 0% | 119 | 8.1 GiB | 9.8 |

Reading the tables: each window has 5,000 cheap requests. *Failed* counts requests a desk
answered with an error or that were still unanswered after 10 s. *Not sent* counts heavy requests
the driver held back because it already had 3,000 waiting on that desk: the desk had no chance
to serve them, so count them against it, and it bounds how long a queue can grow. Rust never
answers a request with an error. What it can't serve it queues, so its overload shows as
latency, memory and *not sent*, where Ash's shows as errors.

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

What it doesn't show: whether a preemptive scheduler matters, and what limited it. It wasn't
the pool or Postgres: doubling the pool changed nothing, and Postgres had only 1.5 to 8 of its
connections busy and its container used about 3 of the 16 cores (see the pool matrix below).
Each server used 8–10 cores, Postgres 3, and the driver and the other applications on the
laptop the rest, which is consistent with the machine as a whole being full. The difference
between the desks is how each treats a full pool: Rust's has no wait limit, Ash's sheds. The
next section takes the database out, and the one after varies the pool's policy. Treat the
figures as a baseline to compare a later change against, and the machine's load, noted in
each run's `manifest.json`, as the largest source of noise.

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
**ash-rust 2,141/s, Ash 456/s**, 4.7 times. Offering both the same heavy rate (each pair of rows is
one load, with the same columns for each desk):

| Heavy offered | | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| none | Rust | 0.9 ms | 3.7 ms | 0% | - | - | - | - | 18 MiB | 0.1 |
|  | Elixir | 0.8 ms | 3.8 ms | 0% | - | - | - | - | 374 MiB | 0.2 |
| 114/s (0.25×) | Rust | 0.8 ms | 4.8 ms | 0% | 5.5 ms | 0% | 0% | 114 | 25 MiB | 0.5 |
|  | Elixir | 0.9 ms | 2.3 ms | 0% | 14.6 ms | 0% | 0% | 114 | 502 MiB | 2.9 |
| 228/s (0.5×) | Rust | 1.0 ms | 4.7 ms | 0% | 5.0 ms | 0% | 0% | 228 | 81 MiB | 0.9 |
|  | Elixir | 0.7 ms | 2.5 ms | 0% | 13.8 ms | 0% | 0% | 228 | 546 MiB | 5.8 |
| 456/s (1×) | Rust | 0.8 ms | 3.8 ms | 0% | 4.7 ms | 0% | 0% | 456 | 84 MiB | 1.9 |
|  | Elixir | 5.9 ms | 32.3 ms | 0% | 903 ms | 0% | 10% | 391 | 5.9 GiB | 13.6 |
| 911/s (2×) | Rust | 0.9 ms | 3.2 ms | 0% | 4.3 ms | 0% | 0% | 911 | 91 MiB | 3.8 |
|  | Elixir | 8.0 ms | 39.0 ms | 0% | 1,261 ms | 0% | 62% | 379 | 6.3 GiB | 13.5 |
| 1,822/s (4×) | Rust | 1.2 ms | 56.2 ms | 0% | 6.2 ms | 0% | 0% | 1,820 | 110 MiB | 10.5 |
|  | Elixir | 8.1 ms | 40.6 ms | 0% | 1,260 ms | 0% | 81% | 387 | 6.5 GiB | 13.7 |
| 3,644/s (8×) | Rust | 201 ms | 605 ms | 0% | 215 ms | 0% | 49% | 2,006 | 134 MiB | 13.3 |
|  | Elixir | 8.1 ms | 40.2 ms | 0% | 1,280 ms | 0% | 91% | 385 | 6.7 GiB | 13.5 |

In each desk's own multiples of its capacity (`--basis own`):

| Heavy offered, × the desk's own capacity | | Offered | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| none | Rust | - | 0.6 ms | 2.7 ms | 0% | - | - | - | - | 18 MiB | 0.0 |
|  | Elixir | - | 0.7 ms | 3.7 ms | 0% | - | - | - | - | 363 MiB | 0.2 |
| 0.5× | Rust | 1,082/s | 0.7 ms | 7.2 ms | 0% | 4.7 ms | 0% | 0% | 1,082 | 98 MiB | 5.0 |
|  | Elixir | 229/s | 0.8 ms | 5.0 ms | 0% | 13.6 ms | 0% | 0% | 229 | 632 MiB | 5.9 |
| 0.9× | Rust | 1,947/s | 98.6 ms | 272 ms | 0% | 101 ms | 0% | <1% | 1,922 | 128 MiB | 11.1 |
|  | Elixir | 413/s | 0.7 ms | 9.9 ms | 0% | 15.1 ms | 0% | 0% | 403 | 1.4 GiB | 11.8 |
| 1× | Rust | 2,163/s | 102 ms | 434 ms | 0% | 105 ms | 0% | 5% | 2,038 | 132 MiB | 11.8 |
|  | Elixir | 459/s | 4.1 ms | 17.5 ms | 0% | 618 ms | 0% | 6% | 419 | 3.5 GiB | 13.5 |
| 1.25× | Rust | 2,704/s | 176 ms | 558 ms | 0% | 185 ms | 0% | 20% | 2,176 | 141 MiB | 13.1 |
|  | Elixir | 573/s | 7.6 ms | 37.5 ms | 0% | 1,242 ms | 0% | 33% | 396 | 6.0 GiB | 13.7 |
| 1.5× | Rust | 3,245/s | 195 ms | 462 ms | 0% | 202 ms | 0% | 35% | 2,183 | 144 MiB | 13.3 |
|  | Elixir | 688/s | 7.7 ms | 46.6 ms | 0% | 1,327 ms | 0% | 50% | 363 | 6.1 GiB | 13.1 |
| 2× | Rust | 4,327/s | 181 ms | 484 ms | 0% | 205 ms | 0% | 54% | 2,165 | 144 MiB | 12.9 |
|  | Elixir | 917/s | 8.0 ms | 42.1 ms | 0% | 1,334 ms | 0% | 63% | 368 | 6.2 GiB | 13.1 |
| 3× | Rust | 6,490/s | 190 ms | 682 ms | 0% | 197 ms | 0% | 70% | 2,149 | 144 MiB | 12.9 |
|  | Elixir | 1,376/s | 8.0 ms | 36.9 ms | 0% | 1,297 ms | 0% | 76% | 376 | 6.3 GiB | 13.5 |

Reading the tables: the cheap stream is 500 requests a second, and nothing failed or timed out on
either desk. *Not sent* counts heavy requests the driver held back because it already had 500
waiting on that desk (the limit this target uses), so a high figure means the desk was holding
that many unanswered: Ash's is 10% at 1× and 91% at 8×, ash-rust's 0% until 8×, where it is 49%.
Those requests are not served, so Ash's *answered* is the better measure of what it did. After the
heavy stream stopped, ash-rust's cheap p99 was back within twice its baseline in 0 to 4 s, Ash's
in 0 to 14 s.

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

### Pool policy (a matrix)

```bash
node bench/saturation.ts --fixture /tmp/fixture.json --matrix        # about 15 minutes
```

`--matrix` runs one 30-second test per configuration at each of two heavy rates (`--rates`,
800 and 1,600 a second, the same for every desk), against pools of 20 and 40 connections
(`--pools`), and samples Postgres during each (its connections in use and its container's CPU).
The desks read their pool from the environment: `POOL_SIZE`, and for Rust `POOL_WAIT_MS`, a new
`ash_postgres::PoolSettings::wait_timeout` that fails a statement that has waited that long
(the default is still no limit); for Elixir `POOL_QUEUE_TARGET_MS`, `POOL_QUEUE_INTERVAL_MS`
and `POOL_TIMEOUT_MS`. Four configurations, at each size:

- **Rust, queues**: the default. A statement waits for a connection as long as it takes.
- **Rust, 100 ms wait limit**: sheds as Ecto does by default.
- **Elixir, sheds**: Ecto's default, a `queue_target` of 50 ms (doubled once the pool has
  been slow for an interval), past which requests are dropped.
- **Elixir, queues**: `queue_target` and `queue_interval` at 60 s, so it no longer drops.

![Pool matrix: cheap latency and failures, heavy answered and failed, memory and Postgres CPU, for each pool policy at 800 and 1,600 heavy requests a second](bench/results/saturation/2026-10-06-pool-matrix/chart.svg)

At 800 heavy requests a second (one rep, 30 s each, with other applications open and a load
average of 16 at the start, so read the differences as indications):

| Configuration (pool 20) | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy p99 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Rust, queues (no wait limit) | 1,454 ms | 1,747 ms | 0% | 4,436 ms | 4,844 ms | 0% | 11% | 632 | 6.5 GiB | 8.5 |
| Elixir, sheds (Ecto's default) | 109 ms | 906 ms | 33% | 423 ms | 1,775 ms | 86% | 0% | 117 | 7.4 GiB | 9.7 |
| Rust, 100 ms wait limit | 99.4 ms | 116 ms | 6% | 311 ms | 338 ms | 22% | 0% | 628 | 853 MiB | 8.4 |
| Elixir, queues (queue_target 60 s) | 3,843 ms | 6,062 ms | 0% | 9,448 ms | 10,013 ms | 36% | 61% | 86 | 22.7 GiB | 10.5 |

What this shows:

- **The pool wasn't the limit, and neither was Postgres.** Pools of 20 and 40 gave the same
  results for every configuration (Rust queueing answered 632 and 625 heavy requests a
  second; Ash shedding, 117 and 116). Postgres had 1.5 to 8 connections active of 20 to 40 and
  its container used 240–420% CPU, 2.4 to 4.2 of 16 cores.
- **A wait limit is what keeps ash-rust's cheap request quick.** At 100 ms, the cheap p99
  is 116 ms against 1.7 s for the unbounded queue, with the same heavy throughput and about an
  eighth of the memory. The cost is the requests that fail: 6% of the cheap ones at 800/s,
  22% at 1,600/s, where heavy throughput also falls (489 to 554 a second against 685 to 696).
- **When both shed at about 100 ms, ash-rust answers about five times as many heavy
  requests** (628 to 689 a second against 116 to 117), with 4–6% of the cheap requests
  failing against 33–34%. Ash answers far fewer than its 265 a second alone. Its pool drops
  a statement at checkout, so a heavy request that has already used CPU can fail partway;
  that is a likely cause and not one measured here.
- **Making Ash queue doesn't make it behave like ash-rust's queue; it's worse.** With the
  queue target at 60 s Ash answered 72 to 86 heavy requests a second (and 61% of its heavy
  requests were *not sent*: the driver already had 3,000 waiting on it), its cheap p99 was 6 s,
  and its memory reached 23 GiB, past the 20 GiB guard that stopped both of those
  configurations from running at 1,600/s. The shedding is what keeps Ash afloat.
- **Ecto's maintainers advise against this.** José Valim: *"You should avoid tweaking
  `queue_target` and `queue_interval` because increasing them mostly means your users have to
  wait longer."* This is what that looks like under overload.

Caveats: the Rust unbounded queue's latency is bounded here by the driver's limit of 3,000
requests in flight (it was holding 3,000 at 1.7 s: with no limit the wait would grow until
requests timed out); one rep per cell; the wait limit surfaces as a pool error, not yet a
typed overload error a server can answer with a 503.

### Equal wait limit

```bash
node bench/saturation.ts --fixture /tmp/fixture.json --matrix --pools 20 --limits 100,250,1000 --rates 400,800,1600
```

`--limits` gives both desks the same limit on how long a request waits for a pool connection, on
20 connections, at 400, 800 and 1,600 heavy requests a second (30 s each). Rust's is
`PoolSettings::wait_timeout`. Ecto has no hard limit: once its pool has been slow for an
interval it drops what has waited past *twice* its `queue_target`, so Elixir is given a
`queue_target` of half the limit and a `queue_interval` of 100 ms.

![Equal wait limit: cheap and heavy latency, failures, throughput and memory, ash-rust and Ash at the same limit on waiting for a connection](bench/results/saturation/2026-10-06-equal-limit/chart.svg)

At 800 heavy requests a second (one rep, with other applications open: read the differences as
indications):

| Limit | | Cheap p50 | Cheap p99 | Cheap failed | Heavy p50 | Heavy p99 | Heavy failed | Heavy not sent | Heavy answered/s | Peak memory | Cores busy |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 100 ms | Rust | 99.5 ms | 117 ms | 7% | 311 ms | 341 ms | 26% | 0% | 591 | 1.5 GiB | 8.0 |
|  | Elixir | 109 ms | 762 ms | 35% | 403 ms | 1,035 ms | 92% | 0% | 62 | 2.7 GiB | 8.4 |
| 250 ms | Rust | 248 ms | 269 ms | 6% | 751 ms | 788 ms | 25% | 0% | 598 | 4.1 GiB | 8.3 |
|  | Elixir | 254 ms | 948 ms | 29% | 833 ms | 1,529 ms | 88% | 0% | 92 | 3.8 GiB | 9.0 |
| 1,000 ms | Rust | 963 ms | 1,019 ms | 5% | 2,885 ms | 3,029 ms | 19% | 0% | 596 | 7.4 GiB | 8.3 |
|  | Elixir | 972 ms | 1,994 ms | 27% | 3,013 ms | 3,805 ms | 84% | 0% | 110 | 9.5 GiB | 9.6 |

What this shows:

- **The limit sets the cheap request's median in both**: p50 is the limit, to within 10%
  (99 and 109 ms at 100 ms; 963 and 972 at 1,000), because the cheap request waits at the pool
  until it's served or dropped.
- **It doesn't set the tail in Elixir.** ash-rust's cheap p99 stays within about 20 ms of the
  limit (117, 269, 1,019); Ash's is 2 to 8 times it (762, 948, 1,994). The pool wait is only part of what
  a request spends in Ash, and the rest isn't bounded by the limit.
- **Heavy requests take about three times the limit** in both (311 and 403 ms at 100 ms; 2,885
  and 3,013 at 1,000), where cheap ones take one. That is consistent with a heavy request
  waiting at the pool about three times, once for each query that follows another; the number of
  queries per request wasn't measured.
- **The same limit doesn't give the same outcome, because Ash saturates first.** At 400 heavy
  requests a second, 57% of ash-rust's capacity and 1.5 times Ash's, ash-rust answers every
  request (cheap p50 0.9 ms, no failures, 4.7 cores, 120–200 MiB) at any limit; Ash fails 13–15% of
  the cheap and 50–56% of the heavy, answers 174–195 heavy requests a second, uses 9.4–9.9 cores
  and 3.3–8.2 GiB. That is about 50 ms of server CPU per heavy request answered against about 12.
  At 800/s ash-rust answers about 600 a second whatever the limit; Ash answers 62 to 110, more as
  the limit grows (it drops fewer, but each costs the same CPU). At 1,600/s ash-rust answers 467 to 545
  and Ash 12 to 16.
- **A longer limit buys ash-rust fewer failures, at the price of latency and memory:** its cheap
  failures fall from 7% to 5% and its heavy from 26% to 19% as the limit goes from 100 ms to 1 s, while
  its cheap p50 goes from 99 ms to 963 ms and its memory from 1.5 to 7.4 GiB. A short limit is the
  better trade for a latency-sensitive request.

Caveats: one rep per cell; Ecto's is an approximation of a hard limit, and a `queue_interval` of
100 ms is shorter than its 2 s default; the two desks, Postgres and the driver share a laptop.
