# ash-sqlite

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-sqlite/badge.svg)](https://docs.rs/ash-sqlite)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-sqlite` is a production-ready relational SQLite data layer for **ash-rust**, powered by `sqlx`.

It translates high-level Ash resource definitions, filters, calculations, and relationships into efficient, parameterized SQL queries with full transactional atomicity and savepoint nesting.

---

## What This Crate Provides

- **`Sqlite` Data Layer**: Implements `ash_core::DataLayer` for SQLite databases.
  - In-memory mode: `Sqlite::memory()`
  - File-backed with Write-Ahead Logging (WAL): `Sqlite::file("path/to/db.sqlite")`
  - Custom connection string: `Sqlite::connect("sqlite://...")`
- **Parameterized SQL Query Compiler**:
  - Compiles `ash_core::Filter` into secure, parameterized `WHERE` clauses using bound values.
  - Inlines calculations directly into SQL expressions.
  - Handles relational queries (`belongs_to`, `has_many`, and `many_to_many` join tables).
  - Evaluates aggregates (`count`, `sum`, `exists`, `first`) via SQL subqueries.
- **Keyset & Offset Pagination in SQL**:
  - Cursor-based `page_keyset` compiles to indexed boundary conditions (`WHERE id > ? ORDER BY id ASC LIMIT ?`).
  - `page_offset` compiles to `LIMIT ? OFFSET ?`.
- **Nested Transactions & Savepoints**:
  - Implements `ash_core::TransactionSupport`.
  - Supports atomic `Multi` pipelines with nested savepoints (`SAVEPOINT ash_tx_...`), guaranteeing complete rollback on error without database corruption.
- **Dynamic Schema Generation**:
  - Implements `ash_core::SchemaSupport` to generate and run DDL tables (`create_table_sql`) directly from static `ResourceDef` attributes.

---

## Quick Example

```rust
use ash_core::{Context, SchemaSupport, resource};
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    resource Customer;
    table "customers";

    attributes {
        id: Uuid [pk],
        name: String,
        email: String,
        balance: i64,
    }

    actions {
        create register {
            primary;
            accept [name, email, balance];
        }

        read read {
            primary;
        }

        update deposit {
            accept [balance];
        }
    }
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    // 1. Connect to an in-memory or file-backed SQLite database
    let db = Sqlite::memory().await?;

    // 2. Automatically generate and install tables
    db.install_resources(&[&Customer::DEF]).await?;

    let ctx = Context::new(db);

    // 3. Create a record
    let customer = Customer::register(&ctx)
        .name("Alice")
        .email("alice@example.com")
        .balance(500)
        .await?;

    // 4. Query with bound SQL filters
    let rich_customers = Customer::query(&ctx)
        .filter(Customer::balance.gte(100))
        .all()
        .await?;

    assert_eq!(rich_customers.len(), 1);
    assert_eq!(rich_customers[0].id, customer.id);

    Ok(())
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
