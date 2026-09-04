# ash-rust

A declarative, resource-oriented framework for Rust inspired by [Elixir's Ash Framework](https://ash-hq.org/).

`ash-rust` models your application as declarative **Resources** encapsulated within bounded **Domains**. It unifies data modeling, relationship graphs, business validations, authorization policies, calculation aggregates, and transactional workflows into a cohesive model that remains decoupled from storage layers and web protocols.

---

## Highlights

- **Declarative DSL (`resource!`)**: Define attributes, actions, relationships, aggregates, calculations, and policies in a unified, readable specification.
- **Pluggable Data Layers**: Resources are storage-agnostic. Run transparently on `ash-memory` for lightning-fast testing or `ash-sqlite` (and future relational engines) for production.
- **Compile-Time Ergonomics**: Procedural macros generate typed action builders (`Order::create(&ctx).amount(100).await`), compile-time checked field accessors (`Order::fields::status`), and fluent filter composition.
- **Bounded Contexts (`domain!`)**: Group resources into business domains that manage cross-resource transactions, schema installations, and typed code interfaces.
- **Ash.Multi & Atomic Transactions**: Compose multi-step pipelines with automatic rollback on error and nested savepoints.
- **Decoupled Extension Architecture**:
  - **Pattern 1: Additive Token-Forwarding (`extend <path>! { ... }`)**: Zero-coupling hooks for companion traits, search indexing, and audit logging.
  - **Pattern 2: Transformative Macro Decorators (`#[transformer] resource! { ... }`)**: Spark-style compile-time AST transformers for state machines, soft delete, and timestamps.
- **Declarative State Machines (`ash-state-machine`)**: Model lifecycles, valid transitions, and state checks with zero boilerplate.
- **Optimistic Concurrency Control**: Built-in `[version]` attributes prevent lost updates with `Error::StaleRecord`.
- **Keyset & Offset Pagination**: Cursor-based keyset pagination (`page_keyset`) and offset pagination (`page_offset`) returning a uniform `Page<T>`.
- **PubSub & Action Notifiers**: Decoupled post-commit event broadcasts with wildcard topic subscriptions (`"orders:*"`, `"order:create"`) and atomic transactional buffering in `Multi`.
- **Resource & Field-Level Authorization**: Role- and actor-based policies with automatic read redaction and mutation enforcement.
- **Declarative Ergonomics**: DRY `accept [field1, field2]`, zero-import field operators (`Resource::field`), fluid `ctx.multi()`, and record lifecycle helpers (`reload`, `destroy`).

---

## Workspace Crates

| Crate | Path | Description |
| :--- | :--- | :--- |
| **`ash-core`** | `crates/ash-core` | Core runtime: resource metadata, changesets, queries, policies, and the `Multi` pipeline engine. |
| **`ash-macros`** | `crates/ash-macros` | Procedural macros providing the declarative `resource!` and `domain!` DSL. |
| **`ash-memory`** | `crates/ash-memory` | Thread-safe, in-memory data layer ideal for unit tests and embedded usage. |
| **`ash-sqlite`** | `crates/ash-sqlite` | Relational SQLite data layer with dynamic SQL generation, savepoints, and relational joins. |
| **`ash-pubsub`** | `crates/ash-pubsub` | Pattern-based PubSub event broker and action notifier for resource broadcasts. |
| **`ash-state-machine`** | `crates/ash-state-machine` | Declarative state machine extension with `#[state_machine]` transformer macro. |
| **`ash-graphql`** | `crates/ash-graphql` | Automatic GraphQL server engine powered by `async-graphql` with dynamic schemas, DataLoader, Relay pagination, and subscriptions. |
| **`ash-sql`** | `crates/ash-sql` | Shared relational query compiler, dialect abstraction, snapshot diffing, and migration engine. |
| **`ash-postgres`** | `crates/ash-postgres` | High-performance PostgreSQL data layer with `RETURNING *` writes, error code mapping, and multitenancy. |
| **`cargo-ash`** | `crates/cargo-ash` | Developer CLI tool for declarative migrations, snapshot dumping, and database management. |

### Examples

- **`kanban`** (`examples/kanban`): A complete multi-domain Trello-like kanban application featuring Workspaces, Boards, Lists, Cards, Comments, and Checklist Items.
- **`helpdesk`** (`examples/helpdesk`): A support ticket tracking domain demonstrating authorization policies, representatives, customer assignments, and calculations.

---

## Quick Start

Add `ash-core` and a data layer to your `Cargo.toml`:

```toml
[dependencies]
ash-core = "0.1"
ash-memory = "0.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
uuid = { version = "1", features = ["v4"] }
```

### 1. Define a Resource

```rust
use ash_core::{Context, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource Task;
    table "tasks";

    attributes {
        id: Uuid [pk],
        title: String,
        completed: bool,
        priority: i64,
        version: i64 [version], // Optimistic concurrency control
    }

    actions {
        create create {
            primary;
            accept [title]; // DRY: inferred from attributes
            change set_attribute(completed, false);
            change set_attribute(priority, 1);
            validation present(title);
            validation string_length(title, min: 3, max: 100);
        }

        read read {
            primary;
        }

        update complete {
            change set_attribute(completed, true);
        }

        update update_priority {
            accept [priority];
            validation numericality(priority, min: 1, max: 5);
        }
    }
}
```

### 2. Execute Actions & Queries

```rust
#[tokio::main]
async fn main() -> ash_core::Result<()> {
    let ctx = Context::new(Memory::new());

    // 1. Create task using generated typed builder
    let task = Task::create(&ctx)
        .title("Write ash-rust docs")
        .await?;

    println!("Created task {} (priority: {}, version: {})", task.title, task.priority, task.version);

    // 2. Query tasks with zero-import field operators
    let pending_tasks = Task::query(&ctx)
        .filter(Task::completed.eq(false))
        .sort_by(Task::priority, true)
        .all()
        .await?;

    assert_eq!(pending_tasks.len(), 1);

    // 3. Update task
    let completed_task = task.complete_on(&ctx).await?;
    assert!(completed_task.completed);
    assert_eq!(completed_task.version, 2);

    // 4. Fluid Multi pipeline (no .changeset(), no .unwrap())
    let batch = ctx.multi()
        .create("task_a", Task::create(&ctx).title("Review PRs"))
        .create("task_b", Task::create(&ctx).title("Deploy release"))
        .commit_without_transaction()
        .await?;
    assert_eq!(batch.len(), 2);

    Ok(())
}
```

---

## Documentation

Comprehensive guides are available in the **[`docs/`](docs/)** directory:

- **[Architecture & Philosophy](docs/architecture.md)**: System design, crate separation, data layer abstractions, and the static Rust compilation model.
- **[Extensibility Guide](docs/extensions.md)**: Pattern 1 (`extend <macro>!`) vs. Pattern 2 (`#[transformer]`), custom validation/change traits, and decoupled error handling.
- **[DSL & Modeling Guide](docs/dsl-guide.md)**: In-depth reference for `resource!`, `domain!`, attributes, identities, embedded resources, timestamps, calculations, aggregates, and actions.
- **[Advanced Capabilities](docs/features.md)**: Ash.Multi transactions, state machines, optimistic locking, query preparations, SQL calculations, multi-store data layer registry, keyset/offset pagination, field-level policies, cascading deletes, managed relationships, and bulk operations.
- **[Authentication & Token Security](docs/wip/0004-ash-authentication.md)**: Declarative authentication strategies (`ash-authentication`), Argon2id password hashing, JWT bearer tokens, API key management, and Axum HTTP extractor.
- **[Performance Benchmarks](docs/benchmarks.md)**: Empirical comparison against canonical Ash Elixir (10–14x throughput speedup, zero GC pressure) and Criterion regression testing.

---

## Running Tests & Benchmarks

Run all unit and integration tests across the workspace:

```bash
cargo test --all
```

Run statistical performance benchmarks with Criterion:

```bash
cargo bench -p helpdesk
```

Run clippy across all targets and features:

```bash
cargo clippy --all-targets --all-features
```

Run the Kanban example CLI:

```bash
cargo run -p kanban -- --help
```

---

## License

This project is licensed under the [MIT License](LICENSE).
