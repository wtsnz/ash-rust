# cargo-ash

`cargo-ash` is the command-line interface for the `ash-rust` ecosystem, providing declarative database schema migrations, schema dumping, and lifecycle management for SQLite and PostgreSQL.

## Features

- **Declarative Migration Generation**: Compares schema snapshots and generates versioned, reversible `.up.sql` and `.down.sql` migration files.
- **Embedded Runner**: Run pending migrations, rollback steps, or inspect migration history.
- **Dual Dialects**: Native support for PostgreSQL (`PostgresDialect`) and SQLite (`SqliteDialect`).
- **Schema Dumps**: Introspect existing databases and export normalized `TableSnapshot` JSON structures.

## Installation

```bash
cargo install --path crates/cargo-ash
```

## Resource codegen

`cargo ash` cannot see your `resource!` definitions. Call `cargo_ash::codegen::main` from a binary in the crate that owns the domain:

```rust
fn main() -> std::process::ExitCode {
    cargo_ash::codegen::main(&[&Helpdesk::DEF])
}
```

```bash
cargo run --bin ash-codegen -- create_helpdesk --dialect postgres
cargo run --bin ash-codegen -- --check
cargo run --bin ash-codegen -- rename_subject --rename tickets.subject=title
cargo run --bin ash-codegen -- rename_tickets --rename-table tickets=issues --rename issues.subject=title
```

When a table disappears and another appears, codegen asks whether it was renamed; in a terminal it prompts, otherwise `--rename-table old=new` answers. Otherwise the new table is created empty and the old one is dropped with its rows. A rename keeps the rows and renames the table's indexes, foreign keys and checks to match. Column renames on a renamed table use the new table name. Without a terminal, an ambiguous column rename is an error until `--rename` answers it.

That writes dialect-suffixed SQL under `migrations/` and snapshots under `resource_snapshots/<dialect>/`. Removed columns stay in the database unless you pass `--drop-columns`. `--check` exits 1 when the resources do not match the snapshots.

`--dev` writes a temporary `{version}_dev.{dialect}.*.sql` pair and updates only `resource_snapshots/<dialect>/dev/`; it takes no migration name. Before a named codegen, roll back each applied `dev` migration newest-first while the down files still exist, then delete those `_dev` SQL files. Named `run` refuses while they remain; once they are gone it writes the squash against committed snapshots.

`--squash` rewrites history as one migration from an empty schema. It refuses while `_ash_schema_migrations` still has rows. Roll back until that table is empty, then pass a migration name with `--squash`. Do not run `cargo ash reset` first. Reset migrates again and leaves tracking rows. Squash also refuses when a `.sql` file in `migrations/` is not a generated `{version}_{name}.{dialect}.up.sql` or `.down.sql` pair. When the gates pass, it writes the new pair, deletes the other generated files for that dialect, and clears `resource_snapshots/<dialect>/dev/`.

```bash
cargo run --bin ash-codegen -- create_schema --squash --dialect postgres
```

Several hosts can migrate one database at once. Each migration runs in its own transaction, which on Postgres first takes `pg_advisory_xact_lock` and on SQLite starts with `BEGIN IMMEDIATE`, then skips the migration if another host already applied it. Rollbacks work the same way and only undo the latest version. The lock ends with the transaction, so it works behind transaction-pooling proxies such as PgBouncer. `Postgres::install` takes the same lock and installs everything in one transaction, so app instances that install on startup do not collide either.

## CLI Usage

```bash
# Generate a new migration
cargo ash migrations generate --name add_priority_to_tickets --dialect postgres

# Run pending migrations
cargo ash migrate --database-url postgres://postgres:postgres@localhost:5432/ash_dev

# Apply all pending migrations
cargo ash setup --database-url postgres://postgres:postgres@localhost:5432/ash_dev

# Roll back every applied migration, then run setup
cargo ash reset --database-url sqlite://ash.db

# Check status of migrations
cargo ash status --database-url sqlite://ash.db

# Rollback latest migration
cargo ash rollback --database-url sqlite://ash.db

# Dump existing database schema into snapshot JSON files
cargo ash dump --database-url postgres://postgres:postgres@localhost:5432/ash_dev --output snapshots/

# Generate TypeScript client SDK and Zod validation schemas
cargo ash codegen ts --out ./frontend/src/ash.ts
# or shortcut:
cargo ash ts -o ./frontend/src/ash.ts
```

## TypeScript Code Generation (`codegen ts`)

`cargo-ash` can inspect your schema snapshots and generate a full TypeScript SDK, Zod form validators, and query builders:

```bash
cargo ash ts \
  --snapshots ./snapshots \
  --out ./frontend/src/ash.ts \
  --client-name AshClient \
  --endpoint /graphql
```

Options:
- `--snapshots <DIR>`: Directory containing snapshot JSON files (default: `snapshots`).
- `--out, -o <FILE>`: Destination TypeScript file (default: `src/ash.ts`).
- `--no-zod`: Skip generating Zod validation schemas.
- `--no-client`: Skip generating the isomorphic client SDK.
- `--no-react`: Skip generating React / TanStack Query helpers.
- `--client-name <NAME>`: Root client class name (default: `AshClient`).
- `--endpoint <URL>`: GraphQL endpoint URL (default: `/graphql`).

