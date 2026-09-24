# ash-postgres

PostgreSQL data layer for `ash-rust` powered by `sqlx`.

## Features

- **Single-Roundtrip Writes**: Leverages PostgreSQL's native `RETURNING *` clause on `INSERT` and `UPDATE` statements to return populated records in a single database roundtrip.
- **SQLSTATE Error Mapping**: Translates native PostgreSQL error codes into Ash domain errors:
  - `23505` $\to$ `Error::IdentityConflict`
  - `23503` $\to$ `Error::DataLayer` (foreign key violation)
  - `23514` $\to$ `Error::Validation` (check constraint)
  - `40P01` $\to$ `Error::DataLayer` (deadlock detected)
- **Schema Multitenancy**: Supports dynamic tenant scoping via `SET LOCAL search_path = <tenant>, public`. Schema migration is `Postgres::migrate_schemas`. There is no `MultitenancyStrategy::Schema`.
- **Nested Transactions**: Full `TransactionSupport` implementation with SQL savepoints (`SAVEPOINT`, `RELEASE SAVEPOINT`, `ROLLBACK TO SAVEPOINT`).
- **Declarative Migrations**: Embedded migration execution via `ash_postgres::migrate` or `cargo ash migrate`.

## Usage

```rust
use ash_postgres::Postgres;
use ash_core::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let db = Postgres::connect("postgres://postgres:postgres@localhost:5432/my_app").await?;
    
    // Install resources or run migrations
    // db.install(&[&MY_RESOURCE]).await?;
    // db.migrate("migrations").await?;
    
    Ok(())
}
```
