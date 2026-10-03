//! SQL for installing resources without migrations.
//!
//! `install()` creates tables directly, so custom statements would otherwise run again on
//! every start. `_ash_statements` records the `up` that ran for each statement, and a
//! statement runs again only when its `up` changes.

/// Creates the table that records installed statements.
pub const CREATE_STATEMENTS_TABLE: &str = "CREATE TABLE IF NOT EXISTS _ash_statements (\
    table_name TEXT NOT NULL, \
    name TEXT NOT NULL, \
    up TEXT NOT NULL, \
    PRIMARY KEY (table_name, name))";

fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// Records a statement that has not run before; affects one row when it should run.
pub fn record_new_statement(table: &str, name: &str, up: &str) -> String {
    format!(
        "INSERT INTO _ash_statements (table_name, name, up) VALUES ({}, {}, {}) \
         ON CONFLICT (table_name, name) DO NOTHING",
        literal(table),
        literal(name),
        literal(up)
    )
}

/// Records a statement whose `up` changed; affects one row when it should run again.
pub fn record_changed_statement(table: &str, name: &str, up: &str) -> String {
    format!(
        "UPDATE _ash_statements SET up = {up} WHERE table_name = {} AND name = {} AND up <> {up}",
        literal(table),
        literal(name),
        up = literal(up)
    )
}

/// Forgets a statement whose `up` failed, so the next install tries it again.
pub fn forget_statement(table: &str, name: &str) -> String {
    format!(
        "DELETE FROM _ash_statements WHERE table_name = {} AND name = {}",
        literal(table),
        literal(name)
    )
}
