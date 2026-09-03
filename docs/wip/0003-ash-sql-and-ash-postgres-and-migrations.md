# RFC 0003: `ash-sql`, `ash-postgres`, and Declarative Resource Migrations

- **Status**: Proposed / WIP
- **Date**: 2026-09-02
- **Authors**: Will, Ash-Rust Team
- **Target Crates**: `ash-sql` (new crate), `ash-postgres` (new crate), `ash-sqlite` (refactored), `ash-core`, `cargo-ash` (CLI tool)

---

## 1. Executive Summary

This RFC specifies the design and implementation of a modular relational database architecture and **declarative schema migration engine** for `ash-rust`, modeling the battle-tested division between **`ash_sql`** and **`ash_postgres`** in the Elixir Ash ecosystem.

### Key Goals
1. **Separation of Concerns (`ash-sql` vs Concrete Adapters)**: Extract all shared relational query compilation, expression translation, lateral join synthesis, keyset pagination, and schema snapshot diffing into a database-agnostic `ash-sql` crate.
2. **First-Class PostgreSQL Support (`ash-postgres`)**: Deliver a production-grade PostgreSQL data layer powered by `sqlx`, featuring native support for PostgreSQL data types (`jsonb`, arrays, `citext`, `pgvector`), schema multitenancy (`search_path`), and PostgreSQL error code mapping.
3. **Declarative Resource Migrations**: Invert traditional migration workflows. Rather than handwriting error-prone SQL migrations and manually keeping Rust structs in sync, **the `resource!` definition is the single source of truth**. A migration generator diffs resource definitions against serializable schema snapshots and automatically produces deterministic, timestamped SQL migration files (`.up.sql` / `.down.sql`).
4. **Universal Ecosystem Interoperability**: Generated migrations are standard `.sql` files that can be run either via an embedded `ash-sql` migration runner or standard ecosystem tooling (`sqlx-cli`, `refinery`, Kubernetes init containers).

---

## 2. Motivation & Ash Elixir Parity

### The Division in Ash Elixir

In early versions of Elixir Ash, all PostgreSQL logic was concentrated in `ash_postgres`. As the ecosystem expanded to support SQLite (`ash_sqlite`) and MySQL, the Ash creators recognized that over 80% of relational SQL generation—such as compiling filter expression trees, planning keyset cursor comparisons, aggregating join rows, and calculating schema differences—was completely dialect-independent.

Ash Elixir factored out **`ash_sql`** as a shared foundational library:

```text
               ┌────────────────────────┐
               │        ash-core        │  (ResourceDef, ActionDef, Expr)
               └───────────▲────────────┘
                           │
               ┌───────────┴────────────┐
               │        ash-sql         │  (Relational Compiler & Migration Diff)
               └─────▲────────────▲─────┘
                     │            │
         ┌───────────┴──┐      ┌──┴───────────┐
         │ ash-postgres │      │  ash-sqlite  │  (Dialects & Drivers)
         └──────────────┘      └──────────────┘
```

### The Problem in Rust Today

In the broader Rust ecosystem, database workflows remain fragmented and manual:
- **Diesel / SeaORM**: Require handwriting migrations (or schema DSLs), maintaining duplicate schema declarations, and manually generating entity files.
- **SQLx**: Provides compile-time query verification against a running database, but leaves schema management and migration authoring entirely to manual SQL scripts.
- **`ash-rust` Current State**: SQL generation is embedded inside `ash-sqlite` using raw string formatting. `ash-sqlite` currently handles schema creation via runtime `CREATE TABLE IF NOT EXISTS`, which does not support schema evolution (e.g. adding/modifying columns, creating indexes, or safe rollbacks in production).

By decoupling SQL generation into `ash-sql` and introducing snapshot-based schema diffing, `ash-rust` brings Elixir's most celebrated superpower to Rust: **zero-boilerplate, declarative database evolution**.

---

## 3. Architecture & Crate Topology

### 3.1 Crate Responsibilities

| Crate | Responsibilities | Dependencies |
|---|---|---|
| **`ash-sql`** | - Shared SQL AST and expression-to-SQL compiler (`Expr` $\rightarrow$ SQL)<br>- Relational query builder (joins, aggregations, keyset pagination)<br>- `SqlDialect` abstraction<br>- Schema snapshot representation (`TableSnapshot`, `ColumnSnapshot`)<br>- Snapshot diffing engine (`diff(old, new) -> Vec<SchemaChange>`)<br>- SQL migration file emitter (`.up.sql` / `.down.sql`)<br>- Built-in lightweight migration runner | `ash-core`, `serde`, `serde_json` |
| **`ash-postgres`** | - `PostgresDialect` implementation for `ash-sql`<br>- `DataLayer` implementation over `sqlx::PgPool`<br>- Native Postgres types (`jsonb`, arrays, `uuid`, `inet`, `pgvector`)<br>- `search_path` schema-based multitenancy<br>- PostgreSQL error code mapping (e.g., `23505` $\rightarrow$ `Error::IdentityConflict`)<br>- `RETURNING *` single-roundtrip writes | `ash-core`, `ash-sql`, `sqlx` (postgres) |
| **`ash-sqlite`** | - Refactored to implement `SqliteDialect` using `ash-sql`<br>- `DataLayer` implementation over `sqlx::SqlitePool`<br>- SQLite specific workarounds (table rebuild for non-additive alters, type affinities) | `ash-core`, `ash-sql`, `sqlx` (sqlite) |
| **`cargo-ash`** | - Developer CLI tool for scaffolding and migration generation (`cargo ash migrations generate`) | `ash-sql`, `clap` |

---

## 4. The Shared Relational Engine: `ash-sql`

### 4.1 The `SqlDialect` Trait

Each concrete database adapter implements `SqlDialect` to customize SQL syntax:

```rust
// in ash-sql/src/dialect.rs

pub trait SqlDialect: Send + Sync + 'static {
    /// Database identifier name (e.g. "postgres", "sqlite").
    fn name(&self) -> &'static str;

    /// Quote an identifier (e.g. `"users"` or `[users]`).
    fn quote_identifier(&self, ident: &str) -> String {
        format!("\"{}\"", ident.replace('"', "\"\""))
    }

    /// SQL parameter placeholder (e.g. `$1` for Postgres, `?` for SQLite).
    fn placeholder(&self, index: usize) -> String;

    /// Map Ash `AttrType` to native SQL type string (e.g. `AttrType::String` -> `VARCHAR(255)` or `TEXT`).
    fn column_type(&self, attr: &AttributeDef) -> String;

    /// Emits `ON CONFLICT (...) DO UPDATE` clause.
    fn upsert_clause(&self, identity: &IdentityDef, update_fields: &[String]) -> String;

    /// Whether the dialect supports `RETURNING *` on INSERT/UPDATE.
    fn supports_returning(&self) -> bool;

    /// Render lateral join syntax for aggregates/relationships.
    fn render_lateral_join(&self, subquery: &str, alias: &str) -> String;
}
```

### 4.2 Query and Expression Compilation

`ash-sql` provides the unified compiler transforming `ash_core::Expr` and `ash_core::Filter` into parameterized SQL:

```rust
// in ash-sql/src/compiler.rs

pub struct SqlParam {
    pub value: Value,
}

pub struct CompiledSql {
    pub sql: String,
    pub params: Vec<SqlParam>,
}

pub struct QueryCompiler<'a, D: SqlDialect> {
    dialect: &'a D,
    param_counter: usize,
    params: Vec<SqlParam>,
}

impl<'a, D: SqlDialect> QueryCompiler<'a, D> {
    pub fn new(dialect: &'a D) -> Self {
        Self { dialect, param_counter: 1, params: Vec::new() }
    }

    pub fn compile_filter(&mut self, filter: &Filter) -> String {
        // Compiles Eq, Gt, Lt, In, Contains, Null checks using dialect placeholders
    }

    pub fn compile_expr(&mut self, expr: &Expr) -> String {
        // Compiles calculations, Coalesce, Case/When, String ops, Math
    }

    pub fn compile_keyset_cursor(&mut self, cursor: &KeysetCursor, sorts: &[Sort]) -> String {
        // Compiles tuple comparison: (col1, id) > ($1, $2)
    }
}
```

---

## 5. Declarative Migrations & Snapshot Diffing

### 5.1 The Resource Schema Snapshot

When `ash-rust` projects evolve, `ash-sql` serializes the structural state of each resource table to a snapshot directory (`.ash/snapshots/<table_name>.json`).

```rust
// in ash-sql/src/snapshot.rs

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableSnapshot {
    pub format_version: u32,
    pub table: String,
    pub schema: Option<String>,
    pub columns: Vec<ColumnSnapshot>,
    pub primary_key: Vec<String>,
    pub identities: Vec<IdentitySnapshot>,
    pub references: Vec<ReferenceSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnSnapshot {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub is_primary_key: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentitySnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceSnapshot {
    pub name: String,
    pub column: String,
    pub target_table: String,
    pub target_column: String,
    pub on_delete: String,
}
```

`ash-core::ResourceDef` gains a converter function:
```rust
impl ResourceDef {
    pub fn to_snapshot<D: SqlDialect>(&self, dialect: &D) -> TableSnapshot {
        // Extracts columns, PK, unique identities, and relationship foreign keys
    }
}
```

### 5.2 The Diffing Algorithm

`ash-sql` compares the existing disk snapshot against the current in-memory `ResourceDef` to produce an ordered sequence of schema operations:

```rust
// in ash-sql/src/diff.rs

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaOperation {
    CreateTable(TableSnapshot),
    DropTable(String),
    AddColumn { table: String, column: ColumnSnapshot },
    DropColumn { table: String, name: String },
    AlterColumnType { table: String, column: String, old_type: String, new_type: String },
    SetNullable { table: String, column: String, nullable: bool },
    SetDefault { table: String, column: String, default: Option<String> },
    CreateIdentity { table: String, identity: IdentitySnapshot },
    DropIdentity { table: String, name: String },
    AddReference { table: String, reference: ReferenceSnapshot },
    DropReference { table: String, name: String },
}

pub fn diff_snapshots(
    old: Option<&TableSnapshot>,
    new: Option<&TableSnapshot>,
) -> Vec<SchemaOperation> {
    // 1. If old is None and new is Some -> CreateTable
    // 2. If old is Some and new is None -> DropTable
    // 3. Compare columns: detect AddColumn, DropColumn, AlterColumnType, SetNullable
    // 4. Compare identities / unique indexes
    // 5. Compare foreign key references
}
```

### 5.3 Ambiguity Resolution (Interactive Renames)

When a column `first_name` disappears and `full_name` appears simultaneously, naive diffing generates:
1. `DROP COLUMN first_name` (destructive data loss)
2. `ADD COLUMN full_name`

The `cargo ash` migration generator detects removed and added columns with compatible types and interactively prompts the developer:
```text
? Table `users`: Column `first_name` was removed, and `full_name` was added.
  [1] Rename `first_name` to `full_name` (preserves data)
  [2] Drop `first_name` and add `full_name` (drops old data)
```
If chosen, it produces `RENAME COLUMN "first_name" TO "full_name"`.

---

## 6. Migration File Generation & Runner

### 6.1 Standard `.sql` File Output

`ash-sql` generates standard timestamped SQL files structured identically to standard industry tooling (e.g. `sqlx migrate`, `refinery`):

```text
migrations/
  20260902204500_create_users_table.postgres.up.sql
  20260902204500_create_users_table.postgres.down.sql
  20260902213000_add_status_to_users.postgres.up.sql
  20260902213000_add_status_to_users.postgres.down.sql
```

#### Example Generated PostgreSQL Migration (`.up.sql`):
```sql
-- Migration: 20260902213000_add_status_to_users.postgres.up.sql
-- Generated automatically by ash-rust. DO NOT EDIT DIRECTLY.

ALTER TABLE "users"
    ADD COLUMN "status" VARCHAR(50) NOT NULL DEFAULT 'active';

CREATE INDEX "users_status_idx" ON "users" ("status");
```

#### Example Generated PostgreSQL Rollback (`.down.sql`):
```sql
-- Migration: 20260902213000_add_status_to_users.postgres.down.sql

DROP INDEX IF EXISTS "users_status_idx";

ALTER TABLE "users"
    DROP COLUMN "status";
```

### 6.2 Embedded Migration Runner

For applications that wish to apply migrations automatically on startup without requiring external CLI tools:

```rust
// in ash-sql/src/migrator.rs

pub struct Migrator<D: SqlDialect> {
    dialect: D,
    migrations_dir: PathBuf,
}

impl<D: SqlDialect> Migrator<D> {
    pub async fn run<'e, E>(&self, executor: E) -> Result<MigrationReport>
    where
        E: sqlx::Executor<'e>,
    {
        // 1. Creates `_ash_schema_migrations` tracking table if missing
        // 2. Reads applied versions
        // 3. Finds unapplied `.up.sql` files sorted by timestamp
        // 4. Executes each migration within an atomic transaction
        // 5. Inserts version into `_ash_schema_migrations`
    }
}
```

---

## 7. `ash-postgres` Data Layer Specification

### 7.1 Connection and Initialization

`ash-postgres` wraps `sqlx::PgPool` and implements `ash_core::DataLayer`:

```rust
use ash_core::{Context, DataLayer};
use ash_postgres::Postgres;

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(20)
        .connect("postgres://postgres:postgres@localhost:5432/myapp")
        .await
        .map_err(|e| ash_core::Error::Invalid(e.to_string()))?;

    let pg = Postgres::new(pool);
    let ctx = Context::new(pg);

    // Context is ready to execute Ash actions directly against Postgres!
    Ok(())
}
```

### 7.2 Native Postgres Capabilities

#### 1. High-Performance Writes with `RETURNING *`
Unlike SQLite which requires separate queries to retrieve generated default values and timestamps, `ash-postgres` appends `RETURNING *` to `INSERT` and `UPDATE` statements, completing the mutation and record retrieval in a **single database roundtrip**:
```sql
INSERT INTO "users" ("id", "email", "created_at", "updated_at")
VALUES ($1, $2, $3, $4)
RETURNING *;
```

#### 2. Native Error Code Translation
PostgreSQL emits standardized error SQLSTATE codes. `ash-postgres` inspects these to surface precise `ash_core::Error` variants:
- **`23505` (unique_violation)**: Parses constraint name to return `Error::IdentityConflict { identity, .. }`.
- **`23503` (foreign_key_violation)**: Returns `Error::Invalid("foreign key violation: referenced record does not exist")`.
- **`23514` (check_violation)**: Returns `Error::ValidationFailed`.
- **`40P01` (deadlock_detected)**: Returns `Error::StaleRecord` or retryable transaction error.

#### 3. Native Postgres Types
`ash-postgres` seamlessly bridges `ash_core::Value` with PostgreSQL types:
- `Value::Map` and `Value::Array` map directly to Postgres `JSONB`.
- `Value::Uuid` maps to native Postgres `UUID`.
- Optional extensions for `citext` and `vector` (via `pgvector`).

#### 4. Schema-Based Multitenancy (`search_path`)
`ash-postgres` supports tenant isolation via PostgreSQL schemas:
```rust
let tenant_ctx = ctx.with_tenant("tenant_acme");
// Automatically prepends: SET LOCAL search_path TO "tenant_acme", "public";
```

---

## 8. End-to-End Developer Workflow

### Step 1: Define Resource in Code
```rust
resource! {
    resource Customer;
    table "customers";
    data_layer postgres;

    attributes {
        id: Uuid [pk],
        email: String,
        plan: String [default: "free"],
    }

    identities {
        identity unique_email: [email];
    }
}
```

### Step 2: Generate Migrations
Developer runs:
```bash
cargo ash migrations generate "create_customers"
```
Output:
```text
[ash] Introspecting resources...
[ash] Diffing against snapshots in .ash/snapshots/...
[ash] Detected new table: `customers`
      + 3 columns (id, email, plan)
      + 1 unique identity (unique_email)
[ash] Writing migrations/20260902204500_create_customers.postgres.up.sql
[ash] Writing migrations/20260902204500_create_customers.postgres.down.sql
[ash] Updated snapshot: .ash/snapshots/customers.json
[ash] Done!
```

### Step 3: Run Migrations
Either via cargo ash:
```bash
cargo ash migrations run
```
Or via standard `sqlx`:
```bash
sqlx migrate run
```
Or automatically on app boot:
```rust
ash_postgres::migrate(&pg_pool).await?;
```

---

## 9. Implementation Roadmap

### Phase 1: `ash-sql` Core Extraction
- [ ] Create `crates/ash-sql`.
- [ ] Define `SqlDialect` trait with default SQL parameter and quoting rules.
- [ ] Move SQL expression and filter compiler from `ash-sqlite` into `ash-sql`.
- [ ] Implement query compiler for lateral joins, sorting, keyset pagination, and aggregates.

### Phase 2: Schema Snapshot & Diff Engine
- [ ] Implement `TableSnapshot`, `ColumnSnapshot`, `IdentitySnapshot` in `ash-sql::snapshot`.
- [ ] Implement `ResourceDef::to_snapshot(dialect)`.
- [ ] Implement `diff_snapshots(old, new) -> Vec<SchemaOperation>` in `ash-sql::diff`.
- [ ] Write unit tests for table creation, column addition, type alteration, nullability change, and index drops.

### Phase 3: SQL Migration Generator & Runner
- [ ] Implement DDL generators for `SchemaOperation` producing `.up.sql` and `.down.sql` strings.
- [ ] Implement `_ash_schema_migrations` tracking table management and file-based `Migrator`.
- [ ] Add integration tests verifying migration run and rollback against in-memory SQLite and PostgreSQL test containers.

### Phase 4: `ash-postgres` Data Layer Crate
- [ ] Create `crates/ash-postgres` wrapping `sqlx::PgPool`.
- [ ] Implement `PostgresDialect` for `ash-sql`.
- [ ] Implement `DataLayer`, `TransactionSupport`, and `SchemaSupport` for `Postgres`.
- [ ] Implement `RETURNING *` optimizations and SQLSTATE error translations.

### Phase 5: Refactor `ash-sqlite`
- [ ] Refactor `ash-sqlite` to depend on `ash-sql`'s `SqliteDialect` and query compiler, eliminating duplicate SQL formatting code.

### Phase 6: `cargo-ash` CLI Tool
- [ ] Create `crates/cargo-ash` CLI with `cargo ash migrations generate` and `cargo ash migrations run`.
- [ ] Implement interactive terminal prompts for column renames vs drop/add.
