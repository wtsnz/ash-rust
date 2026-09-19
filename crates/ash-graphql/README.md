# `ash-graphql`

Automatic GraphQL server engine for `ash-rust` powered by `async-graphql`.

## Overview

`ash-graphql` dynamically transforms Ash Domains and Resources into a production-grade GraphQL API with zero manual boilerplate:

- **Type Reflection**: Resources, attributes, enums, calculations, and aggregates map automatically to GraphQL objects, scalars, and enums.
- **Field-Level Redaction**: Integrates directly with Ash's field policies to redact unauthorized fields to `null`.
- **Query Generation**: Auto-generates type-safe getters (`get<Resource>`) and lists (`list<Resource>s`) with dynamic filtering (`FilterInput`), comparison operators, boolean combinators (`and`, `or`, `not`), and sorting (`SortInput`).
- **Relay Keyset Pagination**: Keyset cursor pagination with `Connection`, `Edge`, and `PageInfo` matching the Relay specification.
- **Mutation Generation**: Resource create, update, and destroy actions map to mutations with structured payloads, action argument validation, optimistic locking, and structured `UserError` responses.
- **DataLoader**: Solves N+1 relationship query problems via batched loading (`AshBatchLoader`) for `belongs_to`, `has_one`, `has_many`, and `many_to_many`.
- **PubSub Subscriptions**: Realtime live event streams hooked directly into `ash-pubsub` (`<resource>Created`, `<resource>Updated`, `<resource>Destroyed`) with predicate filtering.
- **Web Adapters**: First-class Axum integration helpers (`graphql_router`, `graphql_handler`, and interactive `graphiql_handler`).

## Quick Start

```rust,ignore
use ash_graphql::AshGraphQL;
use ash_pubsub::PubSub;
use ash_memory::Memory;

let pubsub = PubSub::new();

// Build schema from an Ash Domain definition
let schema = AshGraphQL::builder(&Helpdesk::DEF)
    .with_pubsub(pubsub.clone())
    .with_dataloader()
    .finish::<Memory>()
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
- `AshEnum` attributes -> GraphQL enums
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
        { priority: { gte: 3 } }
      ]
    }
    sort: [{ field: CREATED_AT, order: DESC }]
    limit: 10
  ) {
    id
    title
  }
}
```

### 3. Relay Keyset Cursor Pagination

Relay-compliant connection queries:
```graphql
query {
  ticketConnection(first: 10, after: "eyJpZCI6...") {
    edges {
      cursor
      node {
        id
        title
      }
    }
    pageInfo {
      hasNextPage
      hasPreviousPage
      startCursor
      endCursor
    }
    totalCount
  }
}
```

### 4. Mutations & Structured Errors

Actions generate mutations with structured `UserError` responses instead of throwing unhandled exceptions:
```graphql
mutation {
  createTicket(input: { title: "Network down", status: OPEN }) {
    success
    errors {
      message
      field
      code
    }
    result {
      id
      title
    }
  }
}
```

### 5. Batched Relationship Loading (DataLoader)

N+1 relationship loading problems are solved using `AshBatchLoader`:
```rust,ignore
let dataloader = AshGraphQL::create_dataloader(ctx.clone(), &[&AUTHOR_DEF, &POST_DEF]);

let req = Request::new(query)
    .data(ctx)
    .data(dataloader);

let res = schema.execute(req).await;
```

### 6. Realtime Subscriptions

Subscribe to resource changes with optional in-memory filter matching:
```graphql
subscription {
  ticketCreated(filter: { status: { eq: URGENT } }) {
    id
    title
    status
  }

  ticketUpdated(id: "...") {
    id
    status
  }

  ticketDestroyed(id: "...")
}
```

### 7. Axum Web Integration

Enable the `axum` feature in `Cargo.toml`:
```toml
[dependencies]
ash-graphql = { path = "...", features = ["axum"] }
```

Mount `graphql_router` to serve `/graphql` and `/graphiql`:
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
- **Relay Keyset Pagination (first: 20)**: ~3,730 ops/sec (~259 µs median)
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
