# ash-postgres

PostgreSQL data layer for `ash-rust`, on `tokio-postgres` with a `deadpool-postgres` pool.

## Features

- **No Round Trips Spent Checking Connections**: A statement checks a connection out of the pool and returns it without pinging the server, as Postgrex doesn't for Ash, so a statement is one round trip. Each statement is prepared once per connection and cached, its parameters declared with their types.
- **Single-Roundtrip Writes**: Leverages PostgreSQL's native `RETURNING *` clause on `INSERT` and `UPDATE` statements to return populated records in a single database roundtrip.
- **SQLSTATE Error Mapping**: Translates native PostgreSQL error codes into Ash domain errors:
  - `23505` $\to$ `Error::IdentityConflict`
  - `23503` $\to$ `Error::DataLayer` (foreign key violation)
  - `23514` $\to$ `Error::Validation` (check constraint)
  - `40P01` $\to$ `Error::DataLayer` (deadlock detected)
- **Schema Multitenancy**: A `strategy: context` resource keeps each tenant's rows in a schema named after the tenant. As with AshPostgres, every statement names the tenant's table (`"acme"."bookings"`), subqueries included, so it works inside and outside transactions and never touches the session's `search_path`. Shared resources stay in the default schema. Create a tenant's tables with `Postgres::install_tenant` or migrate them with `Postgres::migrate_schemas`.
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
