# ash-core

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-core/badge.svg)](https://docs.rs/ash-core)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-core` is the foundational runtime kernel for **ash-rust**, an ergonomic, declarative resource and action framework inspired by Elixir's [Ash Framework](https://ash-hq.org).

It provides the abstractions for static resource metadata, action pipelines, validation and change hooks, query generation, actor authorization policies, and atomic multi-step transactions.

---

## What This Crate Provides

- **Resource Metadata**: Static definitions for attributes, identities, embedded schemas, relationships, calculations, and aggregates via `Resource` and `ResourceDef`.
- **Action Execution Engine**: Lifecycle orchestration for `create`, `read`, `update`, and `destroy` actions with pre-commit validations, changes, and query preparations.
- **Identities & Upsert**: Declarative single or composite unique constraints (`IdentityDef`) and upsert execution.
- **Embedded Resources**: Standalone validation and serialization of nested JSON documents without backing database tables.
- **Timestamps & Dynamic Defaults**: Automated injection and lifecycle updating of `created_at` and `updated_at`, alongside static or dynamic attribute defaults.
- **Changesets**: Safe mutation staging through `Changeset<R>`, enforcing presence, types, lengths, and custom validation logic before touching the data layer.
- **Declarative Authorization**:
  - **Resource Policies**: Actor- and role-based policies (`PolicyDef`, `Check`) compiling down to data-layer read filters. Unauthorized records never load into memory.
  - **Field-Level Policies**: Granular attribute authorization and automatic read redaction via `FieldPolicyDef`.
- **Query & Filter Engine**: Composable, type-safe query builder (`Query<R, D>`) supporting relational joins, aggregations, keyset (`page_keyset`), and offset (`page_offset`) pagination.
- **Preparations Engine**: Query preparations on read actions that inject default filters, sorting, limits, and offsets.
- **Expressions & Calculations**: Rich AST (`Expr`) supporting arithmetic, SQL string functions, coalesce, conditionals, and custom functions.
- **Multi-Store Routing (`DataLayerRegistry`)**: Type-erased multi-backend registry routing operations to SQLite, Memory, or custom stores based on resource declaration.
- **Transactional Pipelines (`Multi`)**: Composable atomic batches inspired by `Ash.Multi`. Steps can create, update, destroy, or compute derived data, guaranteeing all-or-nothing rollback on failure.
- **Bulk & Batch Operations**: High-throughput `bulk_create` and `bulk_destroy` with chunking, upserting, cascading deletes, and chunked query streaming.
- **Declarative Action Lifecycle Hooks**: `before_action`, `after_action`, and `after_transaction` hooks at the action level, via `Change`, and inside `CustomChange` plugins.
- **Managed Relationships**: Declarative and nested writes (`manage_relationship`) across `has_many`, `belongs_to`, and `many_to_many`.
- **Event Notification Primitives**: Core `Notification` payload and `Notifier` trait. Notifications generated during `Multi` pipelines are atomically buffered and only dispatched after database commit.
- **Record Lifecycle Helpers (`ResourceExt`)**: Fluent helpers including `record.reload(&ctx)` and `record.destroy(&ctx)`.

---

## Architectural Role in ash-rust

```text
┌─────────────────────────────────────────────────────────┐
│              ash-macros (resource! / domain!)           │
└───────────────────────────┬─────────────────────────────┘
                            │ generates code using
┌───────────────────────────▼─────────────────────────────┐
│                        ash-core                         │
│   (Engine, Changeset, Query, Policies, Multi, Notifiers)│
└─────────────┬─────────────────────────────┬─────────────┘
              │                             │
    ┌─────────▼─────────┐         ┌─────────▼─────────┐
    │    ash-memory     │         │    ash-sqlite     │
    │ (In-memory layer) │         │  (SQLite backend) │
    └───────────────────┘         └───────────────────┘
```

`ash-core` has minimal external dependencies (`uuid`, `ash-macros`) and no dependencies on specific databases or asynchronous runtimes.

---

## Quick Example

```rust
use ash_core::{Context, Resource, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource Article;
    table "articles";

    attributes {
        id: Uuid [pk],
        title: String,
        views: i64,
        archived: bool,
    }

    actions {
        create publish {
            primary;
            accept [title];
            change set(views = 0);
            change set(archived = false);
            validation present(title);
        }

        read read {
            primary;
        }

        update archive {
            change set(archived = true);
        }

        destroy delete {
            primary;
        }
    }
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    let ctx = Context::new(Memory::new());

    // 1. Create a record using generated action builder
    let article = Article::publish(&ctx)
        .title("Introduction to ash-core")
        .await?;

    // 2. Query with zero-import field operators
    let active = Article::query(&ctx)
        .filter(Article::archived.eq(false) & Article::views.gte(0))
        .all()
        .await?;
    assert_eq!(active.len(), 1);

    // 3. Lifecycle helper: reload
    let reloaded = article.reload(&ctx).await?;
    assert_eq!(reloaded.id, article.id);

    // 4. Atomic Multi batch
    let batch = ctx.multi()
        .create("art2", Article::publish(&ctx).title("Second Post"))
        .commit_without_transaction()
        .await?;
    assert_eq!(batch.len(), 1);

    Ok(())
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
