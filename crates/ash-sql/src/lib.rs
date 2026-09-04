//! # `ash-sql`
//!
//! Shared relational query compilation, SQL dialect abstraction, and declarative schema migrations for `ash-rust`.

pub mod compiler;
pub mod dialect;
pub mod diff;
pub mod generator;
pub mod migrator;
pub mod param;
pub mod snapshot;

pub use compiler::{column, ident, CompiledSql, QueryCompiler};
pub use dialect::{PostgresDialect, SqlDialect, SqliteDialect};
pub use diff::{diff_snapshots, diff_snapshots_with_renames, diff_tables, SchemaOperation};
pub use generator::{
    generate_migration, generate_migration_version, generate_migration_with_version, MigrationFiles,
};
pub use migrator::{MemoryMigrationExecutor, MigrationExecutor, MigrationFile, Migrator};
pub use param::{values_to_json_array, SqlParam};
pub use snapshot::{ColumnSnapshot, IdentitySnapshot, ReferenceSnapshot, TableSnapshot};
