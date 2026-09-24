//! # `ash-sql`
//!
//! Shared relational query compilation, SQL dialect abstraction, and declarative schema migrations for `ash-rust`.

pub mod compiler;
pub mod dialect;
pub mod diff;
pub mod generator;
pub mod migrator;
pub mod param;
pub mod plan;
pub mod snapshot;

pub use compiler::{CompiledSql, QueryCompiler, column, ident};
pub use dialect::{PostgresDialect, SqlDialect, SqliteDialect};
pub use diff::{SchemaOperation, diff_snapshots, diff_snapshots_with_renames, diff_tables};
pub use generator::{
    MigrationFiles, emit_sql, generate_migration, generate_migration_version,
    generate_migration_with_version,
};
pub use migrator::{MemoryMigrationExecutor, MigrationExecutor, MigrationFile, Migrator};
pub use param::{SqlParam, values_to_json_array};
pub use plan::{
    NonInteractive, RenameQuestion, RenameResolver, Resolution, SchemaPlan, plan_schema,
    reverse_plan,
};
pub use snapshot::{
    ColumnSnapshot, IdentitySnapshot, ReferenceSnapshot, TableSnapshot, attribute_sql_default,
    persistable_resources,
};
