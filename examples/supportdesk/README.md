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
