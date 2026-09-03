//! # `ash-sql`
//!
//! Shared relational query compilation, SQL dialect abstraction, and declarative schema migrations for `ash-rust`.

pub mod compiler;
pub mod dialect;
pub mod param;

pub use compiler::{column, ident, CompiledSql, QueryCompiler};
pub use dialect::{PostgresDialect, SqlDialect, SqliteDialect};
pub use param::SqlParam;
