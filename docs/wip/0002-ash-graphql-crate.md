# RFC 0002: `ash-graphql` — Automatic GraphQL Server Engine for `ash-rust`

- **Status**: Proposed / WIP
- **Date**: 2026-09-02
- **Authors**: Will, Ash-Rust Team
- **Target Crates**: `ash-graphql` (new crate), `ash-core`, `ash-pubsub`, `ash-macros`

---

## 1. Executive Summary

This RFC specifies the design and implementation of **`ash-graphql`**, a dedicated extension crate that automatically transforms `ash-rust` **Domains** and **Resources** into a high-performance, specification-compliant **GraphQL server** powered by [`async-graphql`](https://github.com/async-graphql/async-graphql).

Just as `ash_graphql` in the Elixir Ash ecosystem eliminates the need to hand-write Absinthe schemas and resolvers, `ash-graphql` eliminates manual GraphQL plumbing in Rust:
1. **Queries for Read Actions**: Exposes single-record lookups, list queries, and Relay-compliant keyset cursor pagination with filter and sort arguments.
2. **Mutations for Lifecycle Actions**: Automatically builds typed GraphQL mutations for `create`, `update`, and `destroy` actions, including action arguments and structured error payloads.
3. **N+1 Relationship Resolution**: Automatically resolves `belongs_to`, `has_many`, and `many_to_many` relationships using batched `DataLoader` pattern.
4. **Subscriptions Bridging `ash-pubsub`**: Streams live resource notifications over WebSocket connections directly from Ash action events.
5. **Zero-Bypass Policy Enforcement**: Runs every query, mutation, and relationship resolution through `ash-core`'s actor context, policy engine, and field-level redaction.

---

## 2. Motivation & Ash Elixir Parity

### The Problem in Traditional Rust Web Stacks

Building a production GraphQL server in Rust today is notoriously labor-intensive:
- Developers must define separate GraphQL schema structs, database model structs, and input argument structs.
- Resolvers must manually parse inputs, validate constraints, verify actor permissions, coordinate database transactions, and map errors.
- Nested relationships suffer from catastrophic **N+1 query problems** unless developers painstakingly implement `DataLoader` batching for every single relationship.
- Real-time subscriptions require custom WebSocket event loops and manual PubSub forwarding.

### How Ash Solves This

In `ash-rust`, all data model semantics, access controls, actions, and validation logic are already declared in the resource definition:
- `ResourceDef` knows all attributes, relationships, calculations, and aggregates.
- `ActionDef` knows what fields and action arguments are accepted and how inputs must be validated.
- `PolicyDef` and `FieldPolicyDef` define actor-based authorization and field-level redaction rules.
- `DomainDef` groups resources into cohesive bounded contexts.
- `ash-pubsub` provides decoupled real-time notification streams.

By reading these definitions, `ash-graphql` can dynamically construct the entire GraphQL schema at application startup without boilerplate or duplicated code.

---

## 3. Core Architecture & Schema Construction

### 3.1 Dynamic Schema vs. Static Macros

`async-graphql` supports two schema creation approaches:
1. **Static Proc-Macros** (`#[derive(SimpleObject)]`, `#[Object]`): Requires generating thousands of lines of procedural macro token streams at compile-time.
2. **Dynamic Schema** (`async_graphql::dynamic`): Assembles the `async_graphql::dynamic::Schema` at server startup via an imperative, typed builder API.

#### Decision: Use `async_graphql::dynamic`

`ash-graphql` uses `async_graphql::dynamic` for its engine:
- **Compile Time**: Near-zero compile-time overhead compared to massive procedural macro expansion.
- **Architectural Parity**: Mirrors Elixir's runtime reflection of Ash domain/resource definitions.
- **Dynamic Configuration**: Allows runtime feature flags, selective action exposure, and custom resolver overrides without recompilation.

### 3.2 High-Level Architecture Diagram

```
 +-------------------------------------------------------------------------+
 |                                  Client                                 |
 +-------------------------------------------------------------------------+
       | (HTTP POST /graphql)                         | (WebSocket /ws)
       v                                              v
 +-------------------------------------------------------------------------+
 |                    Axum / Actix HTTP Transport Layer                    |
 | - Authenticate actor from JWT/Cookie -> Injects ash_core::Context<D>   |
 +-------------------------------------------------------------------------+
       |                                              |
       v                                              v
 +-------------------------------------------------------------------------+
 |                      async-graphql (Dynamic Schema)                     |
 |                                                                         |
 |  +-----------------------+ +--------------------+ +-------------------+ |
 |  |     QueryRoot         | |    MutationRoot    | | SubscriptionRoot  | |
 |  | - getTicket           | | - createTicket     | | - ticketCreated   | |
 |  | - listTickets         | | - updateTicket     | | - ticketUpdated   | |
 |  | - ticketsConnection   | | - closeTicket      | |                   | |
 |  +-----------------------+ +--------------------+ +-------------------+ |
 +-------------------------------------------------------------------------+
       |                                 |                    ^
       | ash_core::query(...)            | Changeset::commit  | Stream
       v                                 v                    |
 +---------------------------------------------------------+  |
 |                        ash-core                         |  |
 | - Actor Authorization & Policies                        |  |
 | - Field Redaction                                       |  |
 | - Validations, Changes & Optimistic Locking             |  |
 +---------------------------------------------------------+  |
       |                                   |                  |
       v                                   +---> Notifier ----+
 +-------------------------+                       | (ash-pubsub)
 |   Storage Data Layer    |                       v
 |   (SQLite / Memory /    |              +-------------------+
 |    Postgres)            |              | ash_pubsub::PubSub|
 +-------------------------+              +-------------------+
```

---

## 4. Type & Schema Mapping Specification

### 4.1 Attribute to GraphQL Type Mapping

Every attribute on an Ash resource maps to an `async_graphql::dynamic::TypeRef`:

| `ash_core::AttrType` | GraphQL Base Type | Non-Nil Representation (`allow_nil: false`) |
| :--- | :--- | :--- |
| `AttrType::Uuid` | `ID` | `ID!` |
| `AttrType::String` | `String` | `String!` |
| `AttrType::Integer` | `Int` | `Int!` |
| `AttrType::Float` | `Float` | `Float!` |
| `AttrType::Boolean` | `Boolean` | `Boolean!` |
| `AttrType::DateTime` | `String` (or custom `DateTime` scalar) | `String!` |
| `AttrType::Enum` | Custom named `Enum` type | `<Resource><Field>Enum!` |
| `AttrType::Embedded` | Custom named `Object` type (or `JSON`) | `<EmbeddedType>!` |
| `AttrType::Map` | `JSON` scalar | `JSON!` |
| `AttrType::Array` | `[String!]` (or element type) | `[String!]!` |

### 4.2 Enums (`AshEnum`)
Resources using `#[derive(AshEnum)]` have corresponding GraphQL `Enum` types generated automatically:

```graphql
enum TicketStatus {
  OPEN
  IN_PROGRESS
  RESOLVED
  CLOSED
}
```

### 4.3 Calculations & Aggregates as Fields

Ash calculations and aggregates are automatically attached as read-only fields on the resource's GraphQL object:
- **Calculations**: Resolved by evaluating the `ash_core::expr::Expr` or calculation closure against the record fields.
- **Aggregates**: Resolved either via preloaded values or dynamically via data-layer aggregate queries.

```graphql
type Ticket {
  id: ID!
  title: String!
  status: TicketStatus!
  
  # Ash Calculation
  isOverdue: Boolean!
  
  # Ash Aggregate
  commentCount: Int!
  
  # Relationships
  reporter: User!
  comments(limit: Int): [Comment!]!
}
```

### 4.4 Field-Level Policies & Redaction

Whenever an object resolver runs, `ash-graphql` applies the active `Actor` permissions:
1. Field policy evaluations determine read eligibility.
2. If the executing actor lacks read permissions for a field, the field resolves to `null` (or is redacted using `ash_core::policy::redact_fields`), ensuring zero leakage of confidential data.

---

## 5. Query Generation & Relationship Resolution

### 5.1 Single Record Queries

For each primary read action or unique identity, `ash-graphql` registers top-level query fields:

```graphql
type Query {
  # Primary read by ID
  getTicket(id: ID!): Ticket
  
  # Identity lookups
  getUserByEmail(email: String!): User
}
```

### 5.2 List Queries & Filtering

Read actions without arguments (or with filter preparations) expose list queries:

```graphql
type Query {
  listTickets(
    filter: TicketFilterInput
    sort: [TicketSortInput!]
    limit: Int
    offset: Int
  ): [Ticket!]!
}
```

#### Translating GraphQL Filters to `ash_core::filter::Filter`
Input objects mirror Ash filter operators:

```graphql
input TicketFilterInput {
  status: StatusFilterInput
  title: StringFilterInput
  and: [TicketFilterInput!]
  or: [TicketFilterInput!]
}

input StatusFilterInput {
  eq: TicketStatus
  in: [TicketStatus!]
  ne: TicketStatus
}
```

The resolver translates these input structures directly into an `ash_core::filter::Filter` expression tree before executing `ash_core::query(&ctx).filter(...)`.

### 5.3 Relay Keyset Cursor Pagination

For large datasets, `ash-graphql` exposes Relay-compliant connection fields backed by `ash-core`'s keyset cursor engine (`Query::page_keyset`):

```graphql
type Query {
  ticketsConnection(
    first: Int
    after: String
    last: Int
    before: String
    filter: TicketFilterInput
  ): TicketConnection!
}

type TicketConnection {
  edges: [TicketEdge!]!
  pageInfo: PageInfo!
  totalCount: Int
}

type TicketEdge {
  cursor: String!
  node: Ticket!
}

type PageInfo {
  hasNextPage: Boolean!
  hasPreviousPage: Boolean!
  startCursor: String
  endCursor: String
}
```

The base64-encoded `cursor` directly maps to `ash_core::KeysetCursor`, ensuring zero offset-degradation performance at scale.

### 5.4 Relationship Resolution & N+1 Prevention via `DataLoader`

To prevent the classic GraphQL N+1 problem, nested relationship resolvers do **not** run standalone queries per record. Instead, `ash-graphql` uses `async-graphql`'s `DataLoader`.

#### Implementation: `AshBatchLoader<R, D>`

```rust
pub struct AshBatchLoader<D> {
    ctx: ash_core::Context<D>,
}

#[async_trait::async_trait]
impl<D: ash_core::DataLayer> Loader<BatchRelKey> for AshBatchLoader<D> {
    type Value = Vec<ash_core::Value>;
    type Error = Arc<ash_core::Error>;

    async fn load(&self, keys: &[BatchRelKey]) -> Result<HashMap<BatchRelKey, Self::Value>, Self::Error> {
        // Collect all parent IDs and execute a single IN (...) query
        // Groups results by foreign key and returns map
    }
}
```

Nested fields like `ticket { comments { author { name } } }` batch all foreign-key lookups across all items in the query batch into single SQL queries.

---

## 6. Mutations: Create, Update, and Destroy Actions

Each non-read action on a resource generates a dedicated GraphQL mutation following standard Relay mutation conventions.

### 6.1 Mutation Schema Generation

Given a resource action:
```rust
action create :open {
    accept [title, description]
    argument :priority, Priority [optional]
}
```

`ash-graphql` generates:

```graphql
input CreateTicketInput {
  title: String!
  description: String
  priority: Priority
}

type CreateTicketPayload {
  result: Ticket
  errors: [UserError!]!
}

type UserError {
  field: String
  message: String!
  code: String!
}

type Mutation {
  createTicket(input: CreateTicketInput!): CreateTicketPayload!
  updateTicket(id: ID!, input: UpdateTicketInput!): UpdateTicketPayload!
  destroyTicket(id: ID!): DestroyTicketPayload!
}
```

### 6.2 Mutation Execution Pipeline

When a mutation executes:
1. **Input Extraction**: Dynamic values from `ResolverContext` are converted into an `ash_core::FieldMap` and `arguments` map.
2. **Changeset Initialization**: Creates `Changeset::<R>::for_create(&ctx, "open", fields)`.
3. **Optimistic Locking**: If an `expected_version` is supplied, `Changeset::with_optimistic_lock` is verified.
4. **Lifecycle Hooks**: Runs all `before_action`, validations, and changesets inside the context.
5. **Persistence**: Saves the record via the resource's data layer.
6. **Error Mapping**:
   Any error is converted into structured `UserError` entries without crashing the GraphQL response:
   - `Error::Validation { field, message }` $\to$ `{ field: "title", message: "cannot be empty", code: "VALIDATION_ERROR" }`
   - `Error::Forbidden` $\to$ `{ field: null, message: "Action unauthorized by policy", code: "FORBIDDEN" }`
   - `Error::IdentityConflict { .. }` $\to$ `{ field: "email", message: "already taken", code: "CONFLICT" }`
   - `Error::StaleRecord { .. }` $\to$ `{ field: "version", message: "record modified concurrently", code: "STALE_RECORD" }`

---

## 7. Subscriptions: Real-Time Event Streaming

`ash-graphql` provides real-time GraphQL subscriptions by bridging `async-graphql`'s subscription engine to `ash-pubsub`.

### 7.1 Subscription Schema

```graphql
type Subscription {
  ticketCreated: Ticket!
  ticketUpdated(id: ID): Ticket!
  ticketDeleted: ID!
}
```

### 7.2 Bridging `ash-pubsub` to `async-graphql` Stream

In `async-graphql`, dynamic subscription resolvers return a pinned `Stream<Item = ...>`. The subscription resolver bridges from `ash_pubsub::PubSub`:

```rust
use async_stream::stream;
use futures_util::Stream;
use ash_core::Notification;
use ash_pubsub::PubSub;

pub fn resource_subscription_stream(
    pubsub: PubSub,
    topic_pattern: String,
) -> impl Stream<Item = Notification> {
    stream! {
        let mut sub = pubsub.subscribe(topic_pattern);
        while let Ok(notification) = sub.recv().await {
            yield notification;
        }
    }
}
```

When an Ash mutation commits, `ash-core` dispatches the `Notification` to `ash-pubsub`, which pushes the updated record down all active WebSocket client subscriptions.

---

## 8. Declarative Customization DSL (`extend` Pattern)

While `ash-graphql` generates complete schemas by default, applications often require customizing names, hiding actions, or specifying custom pagination styles.

Using **Pattern 1: Token-Forwarding (`extend`)**, resources configure GraphQL settings without coupling `ash-core` to `ash-graphql`:

```rust
resource! {
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        title: String,
        status: TicketStatus [enum, default: TicketStatus::Open],
        secret_notes: Option<String>,
    }

    actions {
        create open { primary, accept: [title] }
        read list { primary }
        update resolve { accept: [status] }
        destroy close { primary }
    }

    // Declarative GraphQL configuration forwarded to ash-graphql
    extend ash_graphql::graphql! {
        type: "Ticket";
        paginate: keyset;
        
        hide_fields: [secret_notes];

        queries {
            get: get_ticket;
            list: list_tickets;
        }

        mutations {
            open: create_ticket;
            resolve: mark_resolved;
            close: destroy_ticket;
        }

        subscriptions {
            on_create: true;
            on_update: true;
        }
    }
}
```

---

## 9. Web Framework Integration (Axum Example)

`ash-graphql` provides pre-built integration helpers for popular web servers like **Axum**.

```rust
use ash_graphql::{AshGraphQL, GraphiQLConfig};
use axum::{
    routing::{get, post},
    Router, Extension,
};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let db = Arc::new(ash_sqlite::Sqlite::connect("app.db").await.unwrap());
    let pubsub = ash_pubsub::PubSub::new();

    // 1. Automatically build GraphQL schema from Ash Domain definition
    let schema = AshGraphQL::builder(&Helpdesk::DEF)
        .with_pubsub(pubsub.clone())
        .with_dataloader()
        .finish();

    // 2. Axum HTTP & WebSocket handlers
    let app = Router::new()
        // GraphiQL Interactive IDE
        .route("/", get(ash_graphql::axum::graphiql_handler("/graphql", "/graphql/ws")))
        // GraphQL Queries & Mutations endpoint
        .route("/graphql", post(ash_graphql::axum::graphql_handler))
        // GraphQL Subscriptions endpoint (WebSocket)
        .route("/graphql/ws", get(ash_graphql::axum::graphql_ws_handler))
        .layer(Extension(schema))
        .layer(Extension(db))
        .layer(Extension(pubsub));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:4000").await.unwrap();
    println!("GraphiQL Playground running at http://127.0.0.1:4000");
    axum::serve(listener, app).await.unwrap();
}
```

### Request Context Extraction (Authentication Middleware)

Inside `graphql_handler`, the current user is extracted from session cookies or JWT tokens and converted to an `ash_core::Actor`:

```rust
async fn graphql_handler(
    Extension(schema): Extension<Schema>,
    Extension(db): Extension<Arc<Sqlite>>,
    actor: Option<Actor>, // extracted via Axum extractor
    req: GraphQLRequest,
) -> GraphQLResponse {
    // Create ash_core::Context with the actor
    let mut ctx = ash_core::Context::new(db);
    if let Some(actor) = actor {
        ctx = ctx.with_actor(actor);
    }
    
    // Inject into async-graphql request
    let request = req.into_inner().data(ctx);
    schema.execute(request).await.into()
}
```

---

## 10. Implementation Plan & Milestones

The implementation of `crates/ash-graphql` will proceed across 6 self-contained phases:

### Phase 1: Crate Setup & Type Reflection
- Create `crates/ash-graphql/Cargo.toml` with `async-graphql = { version = "7", features = ["dynamic-schema"] }`.
- Implement `type_mapping.rs`: convert `AttrType`, `AshEnum`, and embedded resources to `async_graphql::dynamic::TypeRef` and `Object`.
- Support field redaction and policy-aware object field resolution.

### Phase 2: Query Generation & Filtering
- Implement `query_builder.rs`: build `get_<resource>` and `list_<resources>` queries on `QueryRoot`.
- Implement dynamic input objects for `filter` and `sort`.
- Translate GraphQL AST filters into `ash_core::filter::Filter`.

### Phase 3: Keyset Pagination (Relay Connections)
- Implement `connection_builder.rs` generating `Connection`, `Edge`, and `PageInfo` dynamic types.
- Bridge `Query::page_keyset` and `KeysetCursor` into Relay pagination resolvers.

### Phase 4: Mutation Generation & Structured Error Handling
- Implement `mutation_builder.rs`: inspect `create`, `update`, and `destroy` actions.
- Build dynamic `InputObject` types for accepted attributes and action arguments.
- Execute changesets and format failures into standard `UserError` payloads.

### Phase 5: Batched Relationship Loading (N+1 Prevention)
- Implement `dataloader.rs`: create `AshBatchLoader` supporting `belongs_to`, `has_many`, and `many_to_many`.
- Integrate `DataLoader` with dynamic field resolvers.

### Phase 6: Subscriptions via `ash-pubsub` & Web Server Adapters
- Implement `subscription_builder.rs`: build dynamic `Subscription` root.
- Bridge `ash-pubsub` event streams into WebSocket subscription streams.
- Provide Axum / Actix HTTP and WebSocket handlers.
- Write end-to-end integration test verifying Queries, Mutations, Relay Pagination, Policies, and Subscriptions.

---

## 11. Comparison Summary: Elixir vs. Rust

| Feature | Elixir `ash_graphql` | Rust `ash-graphql` |
| :--- | :--- | :--- |
| **Schema Foundation** | Absinthe | `async-graphql` (dynamic engine) |
| **Schema Generation** | Spark DSL compile-time extension | Startup dynamic schema reflection |
| **Queries / Mutations** | Automatic from actions | Automatic from `ActionDef` |
| **N+1 Prevention** | `Dataloader.Ecto` batching | `async-graphql::dataloader::DataLoader` |
| **Relay Pagination** | Keyset pagination with Base64 cursors | Keyset pagination (`ash_core::KeysetCursor`) |
| **Subscriptions** | Absinthe.Subscription via Phoenix PubSub | `async-graphql` subscriptions via `ash-pubsub` |
| **Security & Policies** | Actor passed in Absinthe context | `ash_core::Context<D>` + `Actor` in request data |
| **Compile-Time Impact** | Spark compile-time overhead | Minimal compile overhead via dynamic schema |
