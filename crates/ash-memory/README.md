# ash-memory

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-memory/badge.svg)](https://docs.rs/ash-memory)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-memory` is a thread-safe, purely in-memory data layer for **ash-rust**, inspired by Elixir's `Ash.DataLayer.Ets`.

It enables lightning-fast unit testing, local prototyping, and embedded execution of `ash-core` resources without requiring external database services or disk I/O.

---

## What This Crate Provides

- **`Memory` Data Layer**: Implements `ash_core::DataLayer` backed by `Arc<Mutex<HashMap<String, HashMap<Uuid, FieldMap>>>>>`.
- **Full In-Memory Query Engine**:
  - Filter evaluation (`Eq`, `Gt`, `Gte`, `Lt`, `Lte`, `Nil`, `And`, `Or`, `Not`).
  - Expression and calculations evaluation.
  - Multi-attribute sorting.
  - Relationships resolution (`belongs_to`, `has_many`, `many_to_many` via join tables).
  - Aggregates computation (`count`, `exists`, `sum`, `first`) with custom filter predicates.
  - Keyset (`page_keyset`) and offset (`page_offset`) pagination.
- **Snapshot-Based Atomic Transactions**:
  - Implements `ash_core::TransactionSupport`.
  - Takes an in-memory snapshot before transaction execution.
  - Guarantees full rollback of all state on failure, supporting atomic multi-step `Multi` pipelines.
- **Zero-Op Schema Setup**: Implements `ash_core::SchemaSupport` (no migrations or table initialization needed).

---

## Quick Example

```rust
use ash_core::{Context, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource Item;
    table "items";

    attributes {
        id: Uuid [pk],
        name: String,
        count: i64,
    }

    actions {
        create create {
            primary;
            accept [name, count];
        }

        read read {
            primary;
        }
    }
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    // Initialize an in-memory data layer
    let storage = Memory::new();
    let ctx = Context::new(storage);

    // Create records
    let item = Item::create(&ctx)
        .name("Widget")
        .count(42)
        .await?;

    // Query records in memory
    let results = Item::query(&ctx)
        .filter(Item::count.gte(40))
        .all()
        .await?;

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "Widget");

    Ok(())
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
