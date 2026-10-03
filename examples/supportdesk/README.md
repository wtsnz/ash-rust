# Supportdesk

A multi-tenant support desk, built twice, here on ash-rust and in
[`examples/elixir/supportdesk`](../elixir/supportdesk) on Ash with AshPostgres and
AshGraphql, to benchmark real-world Ash features against each other: policies and field
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
| `edit` | update | accepts `subject`, `priority`, under the `version` optimistic lock |
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

## Parity

`cargo run --release -p supportdesk --bin parity -- --rust http://… --elixir http://…`
sends both apps the benchmark's requests, each as admin, agent and viewer, and compares
the answers: the same records, in the same order, with the same fields visible and the
same errors.
