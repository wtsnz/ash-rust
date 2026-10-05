# `ash-graphql`

Automatic GraphQL server engine for `ash-rust` powered by `async-graphql`.

## Overview

`ash-graphql` dynamically transforms Ash Domains and Resources into a production-grade GraphQL API with zero manual boilerplate.

The schema follows [AshGraphql](https://hexdocs.pm/ash_graphql)'s conventions: the same
names, inputs, pages, mutation results and subscriptions an Elixir Ash app serving the same
resources would have, so one client (such as an `ash-typescript` SDK) works against either.
`examples/cybercab/tests/schema_parity.rs` checks it against an Elixir app's schema.

- **Naming**: fields and arguments in camelCase (`callSign`), enum values upper-cased
  (`OPEN`), root types `RootQueryType`, `RootMutationType` and `RootSubscriptionType`.

- **Type Reflection**: Resources, attributes, enums, calculations, and aggregates map automatically to GraphQL objects, scalars, and enums.
- **Field-Level Redaction**: Integrates directly with Ash's field policies to redact unauthorized fields to `null`.
- **Query Generation**: Auto-generates getters (`get<Resource>`) and keyset-paginated lists (`list<Resource>s`) with filtering (`<Resource>FilterInput`, a `<Resource>Filter<Field>` input per field), boolean combinators (`and`, `or`, `not`), and sorting (`<Resource>SortInput`).
- **Keyset Pagination**: Lists return `KeysetPageOf<Resource> { results, count, startKeyset, endKeyset }`, paged with `first` / `after` and `last` / `before`.
- **Mutation Generation**: Resource create, update, and destroy actions map to mutations taking the record's `id` and an `input`, returning `<Mutation>Result { result, errors }` with Ash's error codes, action argument validation and optimistic locking.
- **DataLoader**: Solves N+1 relationship query problems via batched loading (`AshBatchLoader`) for `belongs_to`, `has_one`, `has_many`, and `many_to_many`.
- **PubSub Subscriptions**: Realtime live event streams hooked directly into `ash-pubsub` (`<resource>Created`, `<resource>Updated`, `<resource>Destroyed`) with predicate filtering.
- **Web Adapters**: First-class Axum integration helpers (`graphql_router`, `graphql_handler`, and interactive `graphiql_handler`).

## Quick Start

```rust,ignore
use ash_graphql::AshGraphQL;
use ash_pubsub::{ContextPubSubExt, PubSub};
use ash_memory::Memory;

let pubsub = PubSub::new();
// Writes publish their changes through the context's notifier.
let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));

// Build schema from an Ash Domain definition
let schema = AshGraphQL::builder(&Helpdesk::DEF)
    .with_pubsub(pubsub.clone())
    .with_dataloader()
    .finish_with_context(ctx)
    .expect("Failed to build GraphQL schema");

// Mount directly onto an Axum router
#[cfg(feature = "axum")]
let app = ash_graphql::axum::graphql_router(schema);
```

## Features

### 1. Type Reflection & Field-Level Redaction

Every Ash attribute maps to an appropriate GraphQL scalar, enum, or list:
- `Uuid` -> `ID!`
- `String` -> `String`
- `Integer` -> `Int`
- `Boolean` -> `Boolean`
- `Map` -> `Json`, output as a JSON object. Input takes an object, or JSON text in a string as AshGraphql's `Json` and `JsonString` take it (AshGraphql's default is `JsonString`, which outputs text; see `examples/supportdesk/GAPS.md` row 30 for why this differs)
- `AshEnum` attributes -> GraphQL enums named after the type, values upper-cased
- Atoms with no enum type -> `String`
- `Calculations` -> Dynamic computation evaluated via `ash_core::eval`
- `Aggregates` -> `count`, `exists`, `sum`, `first`
- `Field Policies` -> Unauthorized fields are automatically redacted to `null`

### 2. Queries & Dynamic Filtering

Generated query fields:
```graphql
query {
  getTicket(id: "...") {
    id
    title
    status
  }

  listTickets(
    filter: {
      and: [
        { status: { eq: OPEN } },
        { priority: { greaterThanOrEqual: 3 } }
      ]
    }
    sort: [{ field: CREATED_AT, order: DESC }]
    first: 10
  ) {
    results {
      id
      title
    }
  }
}
```

Each field's filter takes `eq`, `notEq`, `in`, `lessThan`, `greaterThan`,
`lessThanOrEqual`, `greaterThanOrEqual`, `isNil`, `isDistinctFrom` and
`isNotDistinctFrom`; text fields add `contains`, `stringStartsWith`, `stringEndsWith`,
`like` and `ilike`. `not: [a, b]` excludes records matching all of them.

### 3. Keyset Pagination

Every list pages, as AshGraphql's do: `first` records after `after`, or `last` before
`before`, at most 250; without either, the first 250. Follow `endKeyset` for the rest.
`count` is read only when it's asked for:
```graphql
query {
  listTickets(first: 10, after: "eyJpZCI6...") {
    results {
      id
      title
    }
    startKeyset
    endKeyset
    count
  }
}
```

A read action other than the primary one is served as a plain list, `<action><Resource>s`.

### 4. Mutations & Structured Errors

Actions generate mutations that report failures in `errors` (`MutationError`, with Ash's
codes such as `invalid_attribute`, `required`, `not_found`, `forbidden` and
`stale_record`) instead of raising them. An update or destroy takes the record's `id`; the
`input` is required only when the action requires some of it, and absent when the action
takes none. A destroy's `result` is the record it destroyed. As AshGraphql runs them, an
update or destroy runs by id, as one statement where the data layer can (see
[Atomic updates](../../docs/features.md#3b-atomic-updates)), with no read first:
```graphql
mutation {
  createTicket(input: { title: "Network down", status: OPEN }) {
    result {
      id
      title
    }
    errors {
      message
      code
      fields
    }
  }

  closeTicket(id: "...", input: { resolution: "Rebooted" }) {
    result {
      status
    }
    errors {
      message
    }
  }
}
```

### 5. Batched Relationship Loading (DataLoader)

As AshGraphql does, queries and mutations load the relationships their selection asks
for ahead of the fields: one read per selected relationship, whatever the number of
records, nested selections in turn. A to-many relationship selected with `filter`,
`sort`, `limit` or `offset` loads the same way, with the related query they build: one
read filtered and sorted, each record's rows then limited and offset:
```graphql
{ listAuthors { results { name latest: posts(sort: [{ field: TITLE, order: DESC }], limit: 1) { title } } } }
```

Each read loads only what its selection asks for, as AshGraphql's `select_fields` and
`load_fields` do: the attributes selected, the keys of the relationships selected, and the
aggregates and calculations selected, besides the primary key, the fields a keyset sorts
by, and those field policies check. A mutation's result, and a subscription's record,
load the aggregates, calculations and relationships they select, as AshGraphql loads
them. Reads filter and sort by aggregates too. On Postgres, a relationship's `limit` and
`offset` page each record's rows in the read itself, with a lateral join, as AshPostgres
does; SQLite and memory page each record's rows after one read of them all.

Where a relationship wasn't loaded ahead (a record a custom resolver returns, say), its field loads
it. N+1 relationship loading problems there are solved using `AshBatchLoader`, which loads every
key of a relationship in one read. `with_dataloader()` gives each request its own loader,
bound to the `Context<D>` that request runs as:
```rust,ignore
let schema = AshGraphQL::from_resources(&[&AUTHOR_DEF, &POST_DEF])
    .with_dataloader()
    .finish::<Memory>()?;

// Relationships load as this actor in this tenant, batched.
let req = Request::new(query).data(ctx.with_actor(actor));
let res = schema.execute(req).await;
```

A loader can also be supplied by hand with `AshGraphQL::create_dataloader(ctx)`.
Relationship resolvers only use a loader serving the request's actor and tenant;
otherwise they load directly.

### Reads, Tenancy and Policies

Every read (`get`, `list`, custom read actions, relationships and subscriptions) runs as the request's `Context<D>`, through the same scoping as the typed
API (`ash_core::scope_read`): the read action's preparations and argument filters, the
actor's read policies, and the context's tenant. A record a typed read wouldn't return
isn't reachable over GraphQL either. An `Actor` given in the request data acts for a
context that carries none.

### 6. Realtime Subscriptions

Subscriptions listen on the `PubSub` given to `with_pubsub`. Changes are published to it
only by notifiers: a `PubSubNotifier` on the context a write runs in publishes it once,
after its transaction commits, whether the write came through GraphQL or not. Each
subscriber hears only the records its own read would return, in its tenant.

Subscribe to resource changes, optionally only those matching a filter. Each result holds
the record under `created` or `updated`, or a destroyed record's id under `destroyed`:
```graphql
subscription {
  ticketCreated(filter: { status: { eq: URGENT } }) {
    created {
      id
      title
      status
    }
  }

  ticketUpdated(filter: { id: { eq: "..." } }) {
    updated {
      id
      status
    }
  }

  ticketDestroyed {
    destroyed
  }
}
```

The `PubSub` buffers a bounded number of events per subscriber (`PubSub::with_capacity`),
so a slow client can't grow the server's memory without limit. A subscriber that falls
further behind misses the oldest events. It receives an error with
`extensions: { code: "MISSED_EVENTS", missed: <count> }`, and the subscription ends there.
Clients should resubscribe and re-read whatever they built from its events, as the
`ash-typescript` client does.

### 7. Axum Web Integration

Enable the `axum` feature in `Cargo.toml`:
```toml
[dependencies]
ash-graphql = { path = "...", features = ["axum"] }
```

Mount `graphql_router` to serve `/graphql`, `/graphiql`, and subscriptions over
WebSocket at `/graphql/ws` (`graphql-transport-ws` or the older `graphql-ws` protocol):
```rust,ignore
use axum::Router;
use ash_graphql::axum::graphql_router;

let app = graphql_router(schema);
let listener = tokio::net::TcpListener::bind("0.0.0.0:4000").await.unwrap();
axum::serve(listener, app).await.unwrap();
```

## Benchmarking & Performance

`ash-graphql` includes both an industry-standard statistical Criterion suite and a fast standalone runner for real-world API performance profiling:

### 1. Fast Standalone Runner
```bash
cargo run --release -p ash-graphql --example bench_graphql --features axum
```

Measures throughput (ops/sec), average, median (p50), p95, and p99 latency across realistic workloads:
- **Schema Build (Dynamic Reflection)**: ~6,700 ops/sec (~140 µs median)
- **Single Record by ID**: ~14,800 ops/sec (~65 µs median)
- **100 Tickets Collection Query**: ~2,570 ops/sec (~377 µs median)
- **Filtered & Sorted Query (50 items)**: ~4,510 ops/sec (~217 µs median)
- **Keyset Pagination (first: 20)**: ~3,730 ops/sec (~259 µs median)
- **DataLoader (100 Tickets + Nested Author)**: ~580 ops/sec (~1.7 ms median, batching 100 queries into 1)
- **Axum HTTP POST `/graphql` Roundtrip (20 items)**: ~7,530 ops/sec (~125 µs median)
- **GraphQL Mutation `openTicket` (Validation + Action)**: ~31,100 ops/sec (~31 µs median)
- **SQLite DataLayer (100 Tickets Query)**: ~1,470 ops/sec (~653 µs median)

### 2. Criterion Statistical Regression Suite
```bash
cargo bench -p ash-graphql --bench graphql_bench --features axum
```

To run a fast smoke verification:
```bash
cargo bench -p ash-graphql --bench graphql_bench --features axum -- --test
```
