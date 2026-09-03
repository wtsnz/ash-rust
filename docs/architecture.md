# Architecture & Philosophy

`ash-rust` brings the declarative, resource-centric philosophy of Elixir's Ash Framework to Rust. Rather than scattering business rules across controller handlers, ORM hooks, and ad-hoc service layers, `ash-rust` centralizes all behavior on **Resources**.

---

## 1. Core Principles

1. **Declarative Specification**: Business rules, state validations, relationship integrity, calculations, and authorization policies are declared on the resource itself.
2. **Pluggable Data Layers**: Resources are storage-agnostic. A resource can run against an in-memory test database, SQLite, Postgres, or an external API without changing resource definitions or application code.
3. **Decoupled Extensibility**: `ash-core` remains lean and minimal. Features like state machines, audit trails, and search indexing live in separate crates and hook into open extension points.
4. **Compile-Time Safety & Zero-Cost Ergonomics**: Macros generate typed action builders (`Post::create(&ctx).title("...").await`), fluent query filters, and compile-time checked relationships.

---

## 2. Crate Organization

The project is organized into modular crates with strict dependency directions:

```text
               ┌────────────────────────┐
               │    Application / App   │
               └───────────┬────────────┘
                           │
         ┌─────────────────┼─────────────────┬─────────────────┐
         ▼                 ▼                 ▼                 ▼
┌─────────────────┐ ┌─────────────┐ ┌───────────────────┐ ┌──────────────┐
│ash-state-machine│ │ ash-sqlite  │ │    ash-memory     │ │  ash-pubsub  │
└────────┬────────┘ └──────┬──────┘ └─────────┬─────────┘ └──────┬───────┘
         │ (uses traits)   │ (DataLayer)      │ (DataLayer)      │ (Notifier)
         ▼                 ▼                  ▼                  ▼
┌────────────────────────────────────────────────────────────────────────┐
│                               ash-core                                 │
│ (ResourceDef, ActionDef, Changeset, Query, Multi, Notifier)            │
└───────────────────────────────────▲────────────────────────────────────┘
                                    │ (generates code for)
┌───────────────────────────────────┴────────────────────────────────────┐
│                              ash-macros                                │
│                       (resource!, domain! DSL)                         │
└────────────────────────────────────────────────────────────────────────┘
```

### `crates/ash-core`
- The runtime foundation.
- Defines core structs: `ResourceDef`, `ActionDef`, `AttributeDef`, `RelationshipDef`, `CalculationDef`, `AggregateDef`, `PolicyDef`, `FieldPolicyDef`.
- Defines lifecycle notifications: `Notification` payload, `Notifier` trait, `SyncFnNotifier`, and transaction buffering in `Multi`.
- Defines execution engines: `Changeset`, `Query`, `Multi` (atomic transaction pipeline), `Context<D>`, `Actor`.
- Defines decoupling extension traits: `DataLayer`, `ResourceExtension`, `CustomValidation`, `CustomChange`, `Notifier`.
- **Zero knowledge of third-party features** like state machines, PubSub brokers, or SQL dialects.

### `crates/ash-macros`
- The procedural macro engine providing the declarative DSL (`resource!` and `domain!`).
- Parses resource definitions into an AST and generates:
  - Strong Rust structs representing the resource.
  - Const `DEF: ResourceDef` runtime metadata.
  - Action builder structs for primary/custom creates, updates, and destroys.
  - Nested `fields` module for compile-time typed attribute and aggregate accessors.
  - Token-forwarding handlers (`extend <macro>! { ... }`).

### `crates/ash-memory`
- In-memory data layer implementing the `DataLayer` trait.
- Backed by thread-safe HashMaps/BTreeMaps.
- Ideal for fast unit testing and microservices without database dependencies.

### `crates/ash-sqlite`
- Production-grade SQLite data layer implementing `DataLayer` and `TransactionSupport`.
- Features dynamic SQL generation via `sqlx::QueryBuilder`, parameterized queries, savepoints for nested transactions, and SQL `JOIN` resolution for many-to-many relationships and aggregates.

### `crates/ash-pubsub`
- Standalone external extension crate providing a pattern-based, in-memory PubSub broker and `PubSubNotifier`.
- Implements `ash_core::Notifier` to broadcast resource mutations to wildcard subscriptions (`orders:*`, `orders:create`) with deduplicated multi-topic delivery.
- Mirrors Elixir's `ash_pubsub` package: `ash-core` defines the notification contract, and `ash-pubsub` handles channel routing and subscriber streams.

### `crates/ash-state-machine` & `crates/ash-state-machine-macros`
- Standalone external extension crate providing declarative state machines.
- Proves the decoupling architecture: `ash-core` has zero dependencies on `ash-state-machine`.

---

## 3. Spark DSL (Elixir) vs. Rust Macro Engine

In Elixir, Ash leverages the **Spark DSL**, which uses runtime module reflection and compile-time transformer phases (`Spark.Dsl.Transformer`) to dynamically alter modules and validate options.

Rust's compilation model is strictly static:
- Crate dependencies form a directed acyclic graph (DAG).
- Procedural macros transform token streams at parse time; they cannot inspect compiled types or other crates dynamically.

To deliver Elixir's elegance within Rust's static constraints, `ash-rust` employs two decoupled architectural patterns:
1. **Additive Token-Forwarding (`extend`)**: For companions, traits, and schemas that do not alter core resource definitions.
2. **Transformative Macro Decorators (`#[transformer]`)**: For extensions like state machines that rewrite the resource AST prior to `resource!` expansion.

See the **[Extensibility Guide](extensions.md)** for detailed coverage.
