# ash-sql

Shared relational query compiler, dialect abstraction, and declarative migration engine for `ash-rust`.

## Architecture

`ash-sql` acts as the shared engine powering both `ash-sqlite` and `ash-postgres`:

- **`SqlDialect`**: Extensible trait defining database dialect specifics (identifiers, parameter placeholders, column type mappings, UPSERT syntax, RETURNING support).
  - `SqliteDialect`
  - `PostgresDialect`
- **`QueryCompiler`**: Compiles Ash `ResourceDef`, `Filter`, `Expr`, `Sort`, and `KeysetCursor` into dialect-specific parameterized SQL (`CompiledSql`).
- **`TableSnapshot`**: Serializable schema representation capturing tables, columns, nullability, defaults, primary keys, identities, and foreign references.
- **`diff_snapshots` / `diff_tables`**: Declarative schema diffing engine calculating structural operations (`CreateTable`, `DropTable`, `AddColumn`, `DropColumn`, `AlterColumnType`, `SetNullable`, etc.).
- **`generate_migration`**: Generates timestamped, reversible `.up.sql` and `.down.sql` scripts.
- **`Migrator`**: Tracks and applies versioned migrations against any `MigrationExecutor` using an `_ash_schema_migrations` tracking table.
