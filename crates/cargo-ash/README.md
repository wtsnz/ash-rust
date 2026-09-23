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
```

That writes dialect-suffixed SQL under `migrations/` and snapshots under `resource_snapshots/<dialect>/`. Removed columns stay in the database unless you pass `--drop-columns`. `--check` exits 1 when the resources do not match the snapshots.

## CLI Usage

```bash
# Generate a new migration
cargo ash migrations generate --name add_priority_to_tickets --dialect postgres

# Run pending migrations
cargo ash migrate --database-url postgres://postgres:postgres@localhost:5432/ash_dev

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

