//! # `ash-sql`
//!
//! Shared relational query compilation, SQL dialect abstraction, and declarative schema migrations for `ash-rust`.

pub mod compiler;
pub mod dialect;
pub mod diff;
pub mod param;
pub mod snapshot;

pub use compiler::{column, ident, CompiledSql, QueryCompiler};
pub use dialect::{PostgresDialect, SqlDialect, SqliteDialect};
pub use diff::{diff_snapshots, diff_snapshots_with_renames, diff_tables, SchemaOperation};
pub use param::SqlParam;
pub use snapshot::{ColumnSnapshot, IdentitySnapshot, ReferenceSnapshot, TableSnapshot};
