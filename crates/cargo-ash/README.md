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
```
