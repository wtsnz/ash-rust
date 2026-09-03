# `ash-graphql`

Automatic GraphQL server engine for `ash-rust` powered by `async-graphql`.

## Overview

`ash-graphql` transforms Ash Domains and Resources into a fully-functional GraphQL API:

- **Type Reflection**: Resources, attributes, enums, calculations, and aggregates map automatically to GraphQL objects, scalars, and enums.
- **Field-Level Redaction**: Integrates directly with Ash's field policies to redact unauthorized fields to `null`.
- **Query Generation**: Auto-generates type-safe getters and lists with dynamic filters and sorting.
- **Relay Keyset Pagination**: Keyset cursor pagination with `Connection`, `Edge`, and `PageInfo`.
- **Mutation Generation**: Resource actions map to mutations with structured payloads and user error handling.
- **DataLoader**: Solves N+1 relationship query problems via batched loading.
- **PubSub Subscriptions**: Live event broadcasting hooked directly into `ash-pubsub`.
- **Web Adapters**: First-class Axum integration helpers.

## Quick Start

```rust,ignore
use ash_graphql::AshGraphQL;

let schema = AshGraphQL::builder(&Helpdesk::DEF)
    .with_pubsub(pubsub)
    .with_dataloader()
    .finish()?;
```
