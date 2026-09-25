use std::collections::HashSet;

use crate::dialect::SqlDialect;
use crate::diff::SchemaOperation;
use crate::snapshot::TableSnapshot;

/// Contains generated `.up.sql` and `.down.sql` migration files metadata and contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationFiles {
    pub version: String,
    pub name: String,
    pub up_filename: String,
    pub down_filename: String,
    pub up_sql: String,
    pub down_sql: String,
}

/// Generates a timestamp string in the standard `YYYYMMDDHHMMSS` format.
pub fn generate_migration_version() -> String {
    let now = std::time::SystemTime::now();
    let total_secs = now
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let sec = (total_secs % 60) as u32;
    let total_mins = total_secs / 60;
    let min = (total_mins % 60) as u32;
    let total_hours = total_mins / 60;
    let hour = (total_hours % 24) as u32;
    let total_days = (total_hours / 24) as i64;

    let z = total_days + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}{m:02}{d:02}{hour:02}{min:02}{sec:02}")
}

/// Generates SQL migration up and down scripts for a set of schema operations.
pub fn generate_migration<D: SqlDialect>(
    dialect: &D,
    name: &str,
    operations: &[SchemaOperation],
) -> MigrationFiles {
    let version = generate_migration_version();
    generate_migration_with_version(dialect, &version, name, operations)
}

/// Generates SQL migration scripts with an explicit timestamp version.
pub fn generate_migration_with_version<D: SqlDialect>(
    dialect: &D,
    version: &str,
    name: &str,
    operations: &[SchemaOperation],
) -> MigrationFiles {
    let dialect_name = dialect.name();
    let up_filename = format!("{version}_{name}.{dialect_name}.up.sql");
    let down_filename = format!("{version}_{name}.{dialect_name}.down.sql");

    let mut up_stmts = Vec::new();
    let mut down_stmts = Vec::new();

    for op in operations {
        let (up, down) = generate_operation_sql(dialect, op);
        if !up.is_empty() {
            up_stmts.push(up);
        }
        if !down.is_empty() {
            down_stmts.push(down);
        }
    }

    // Down statements should be executed in reverse order
    down_stmts.reverse();

    let up_header = format!(
        "-- Migration: {up_filename}\n-- Generated automatically by ash-rust. DO NOT EDIT DIRECTLY.\n\n"
    );
    let down_header = format!(
        "-- Migration: {down_filename}\n-- Generated automatically by ash-rust. DO NOT EDIT DIRECTLY.\n\n"
    );

    let up_sql = format!("{up_header}{}", up_stmts.join("\n\n"));
    let down_sql = format!("{down_header}{}", down_stmts.join("\n\n"));

    MigrationFiles {
        version: version.to_string(),
        name: name.to_string(),
        up_filename,
        down_filename,
        up_sql,
        down_sql,
    }
}

fn generate_operation_sql<D: SqlDialect>(dialect: &D, op: &SchemaOperation) -> (String, String) {
    match op {
        SchemaOperation::CreateTable(snapshot) => {
            let mut up = emit_create_table(dialect, snapshot);
            up.push_str(&emit_indexes(dialect, snapshot));
            let table = dialect.quote_identifier(&snapshot.table);
            let down = format!("DROP TABLE IF EXISTS {table};");
            (up, down)
        }
        SchemaOperation::DropTable(table_name) => {
            let table = dialect.quote_identifier(table_name);
            let up = format!("DROP TABLE IF EXISTS {table};");
            let down = format!("-- Rollback for dropped table {table}");
            (up, down)
        }
        SchemaOperation::AddColumn { table, column } => {
            let t = dialect.quote_identifier(table);
            let col_name = dialect.quote_identifier(&column.name);
            let mut def = format!("{col_name} {}", column.sql_type);
            if !column.nullable {
                def.push_str(" NOT NULL");
            }
            if let Some(default) = &column.default {
                def.push_str(&format!(" DEFAULT {default}"));
            }
            let up = format!("ALTER TABLE {t} ADD COLUMN {def};");
            let down = format!("ALTER TABLE {t} DROP COLUMN {col_name};");
            (up, down)
        }
        SchemaOperation::DropColumn { table, name } => {
            let t = dialect.quote_identifier(table);
            let col_name = dialect.quote_identifier(name);
            let up = format!("ALTER TABLE {t} DROP COLUMN {col_name};");
            let down = format!("-- Rollback for dropped column {col_name} on {t}");
            (up, down)
        }
        SchemaOperation::RenameTable { old_name, new_name } => {
            let old_table = dialect.quote_identifier(old_name);
            let new_table = dialect.quote_identifier(new_name);
            let up = format!("ALTER TABLE {old_table} RENAME TO {new_table};");
            let down = format!("ALTER TABLE {new_table} RENAME TO {old_table};");
            (up, down)
        }
        SchemaOperation::RenameColumn {
            table,
            old_name,
            new_name,
        } => {
            let t = dialect.quote_identifier(table);
            let o = dialect.quote_identifier(old_name);
            let n = dialect.quote_identifier(new_name);
            let up = format!("ALTER TABLE {t} RENAME COLUMN {o} TO {n};");
            let down = format!("ALTER TABLE {t} RENAME COLUMN {n} TO {o};");
            (up, down)
        }
        SchemaOperation::AlterColumnType {
            table,
            column,
            old_type,
            new_type,
        } => {
            let t = dialect.quote_identifier(table);
            let col = dialect.quote_identifier(column);
            if dialect.name() == "postgres" {
                let up = format!(
                    "ALTER TABLE {t} ALTER COLUMN {col} TYPE {new_type} USING {col}::{new_type};"
                );
                let down = format!(
                    "ALTER TABLE {t} ALTER COLUMN {col} TYPE {old_type} USING {col}::{old_type};"
                );
                (up, down)
            } else {
                let up = format!("-- SQLite does not support ALTER COLUMN TYPE for {col} on {t}");
                let down = format!("-- Rollback ALTER COLUMN TYPE for {col} on {t}");
                (up, down)
            }
        }
        SchemaOperation::SetNullable {
            table,
            column,
            nullable,
        } => {
            let t = dialect.quote_identifier(table);
            let col = dialect.quote_identifier(column);
            if dialect.name() == "postgres" {
                let (up_clause, down_clause) = if *nullable {
                    ("DROP NOT NULL", "SET NOT NULL")
                } else {
                    ("SET NOT NULL", "DROP NOT NULL")
                };
                let up = format!("ALTER TABLE {t} ALTER COLUMN {col} {up_clause};");
                let down = format!("ALTER TABLE {t} ALTER COLUMN {col} {down_clause};");
                (up, down)
            } else {
                let up =
                    "-- SQLite does not support modifying column nullability directly".to_string();
                let down = "-- Rollback column nullability modification".to_string();
                (up, down)
            }
        }
        SchemaOperation::SetDefault {
            table,
            column,
            default,
        } => {
            let t = dialect.quote_identifier(table);
            let col = dialect.quote_identifier(column);
            if dialect.name() == "postgres" {
                let up = match default {
                    Some(d) => format!("ALTER TABLE {t} ALTER COLUMN {col} SET DEFAULT {d};"),
                    None => format!("ALTER TABLE {t} ALTER COLUMN {col} DROP DEFAULT;"),
                };
                let down = format!("-- Rollback SET DEFAULT for {col} on {t}");
                (up, down)
            } else {
                let up = "-- SQLite does not support modifying column default directly".to_string();
                let down = "-- Rollback column default modification".to_string();
                (up, down)
            }
        }
        SchemaOperation::CreateIdentity { table, identity } => {
            let t = dialect.quote_identifier(table);
            let id_name = dialect.quote_identifier(&identity.name);
            let key_cols = identity
                .columns
                .iter()
                .map(|k| dialect.quote_identifier(k))
                .collect::<Vec<_>>()
                .join(", ");
            let up = format!(
                "{};",
                format_create_index(
                    true,
                    true,
                    &id_name,
                    &t,
                    &key_cols,
                    identity.predicate.as_deref(),
                    dialect.name(),
                    identity.nils_distinct,
                    None,
                )
            );
            let down = format!("DROP INDEX IF EXISTS {id_name};");
            (up, down)
        }
        SchemaOperation::DropIdentity { table: _, name } => {
            let id_name = dialect.quote_identifier(name);
            let up = format!("DROP INDEX IF EXISTS {id_name};");
            let down = format!("-- Rollback DROP INDEX {id_name}");
            (up, down)
        }
        SchemaOperation::CreateIndex { table, index } => {
            let t = dialect.quote_identifier(table);
            let idx_name = dialect.quote_identifier(&index.name);
            let key_cols = index
                .columns
                .iter()
                .map(|k| dialect.quote_identifier(k))
                .collect::<Vec<_>>()
                .join(", ");
            let up = format!(
                "{};",
                format_create_index(
                    false,
                    true,
                    &idx_name,
                    &t,
                    &key_cols,
                    index.predicate.as_deref(),
                    dialect.name(),
                    true,
                    index.method.as_deref(),
                )
            );
            let down = format!("DROP INDEX IF EXISTS {idx_name};");
            (up, down)
        }
        SchemaOperation::DropIndex { table: _, name } => {
            let idx_name = dialect.quote_identifier(name);
            let up = format!("DROP INDEX IF EXISTS {idx_name};");
            let down = format!("-- Rollback DROP INDEX {idx_name}");
            (up, down)
        }
        SchemaOperation::AddCheck { table, check } => {
            let t = dialect.quote_identifier(table);
            let ck_name = dialect.quote_identifier(&check.name);
            let up = format!(
                "ALTER TABLE {t} ADD CONSTRAINT {ck_name} CHECK ({});",
                check.expression
            );
            let down = format!("ALTER TABLE {t} DROP CONSTRAINT IF EXISTS {ck_name};");
            (up, down)
        }
        SchemaOperation::DropCheck { table, name } => {
            let t = dialect.quote_identifier(table);
            let ck_name = dialect.quote_identifier(name);
            let up = format!("ALTER TABLE {t} DROP CONSTRAINT IF EXISTS {ck_name};");
            let down = format!("-- Rollback DROP CONSTRAINT {ck_name} on {t}");
            (up, down)
        }
        SchemaOperation::AddReference { table, reference } => {
            let t = dialect.quote_identifier(table);
            let ref_name = dialect.quote_identifier(&reference.name);
            let (col, target_col) = reference.key_sql(dialect);
            let target_t = dialect.quote_identifier(&reference.target_table);
            let on_del = &reference.on_delete;

            let up = format!(
                "ALTER TABLE {t} ADD CONSTRAINT {ref_name} FOREIGN KEY ({col}) REFERENCES {target_t} ({target_col}) ON DELETE {on_del}{};",
                reference.on_update_sql()
            );
            let down = format!("ALTER TABLE {t} DROP CONSTRAINT IF EXISTS {ref_name};");
            (up, down)
        }
        SchemaOperation::DropReference { table, name } => {
            let t = dialect.quote_identifier(table);
            let ref_name = dialect.quote_identifier(name);
            let up = format!("ALTER TABLE {t} DROP CONSTRAINT IF EXISTS {ref_name};");
            let down = format!("-- Rollback DROP CONSTRAINT {ref_name} on {t}");
            (up, down)
        }
    }
}

pub fn emit_create_table<D: SqlDialect>(dialect: &D, snapshot: &TableSnapshot) -> String {
    let table = dialect.quote_identifier(&snapshot.table);
    let mut cols = Vec::new();

    for col in &snapshot.columns {
        let col_name = dialect.quote_identifier(&col.name);
        let mut def = format!("{col_name} {}", col.sql_type);
        if col.is_primary_key {
            def.push_str(" PRIMARY KEY");
        }
        if !col.nullable && !col.is_primary_key {
            def.push_str(" NOT NULL");
        }
        if let Some(default) = &col.default {
            def.push_str(&format!(" DEFAULT {default}"));
        }
        cols.push(def);
    }

    for check in &snapshot.checks {
        let ck_name = dialect.quote_identifier(&check.name);
        cols.push(format!("CONSTRAINT {ck_name} CHECK ({})", check.expression));
    }

    for reference in &snapshot.references {
        let ref_name = dialect.quote_identifier(&reference.name);
        let (col, target_col) = reference.key_sql(dialect);
        let target_t = dialect.quote_identifier(&reference.target_table);
        cols.push(format!(
            "CONSTRAINT {ref_name} FOREIGN KEY ({col}) REFERENCES {target_t} ({target_col}) ON DELETE {}{}",
            reference.on_delete,
            reference.on_update_sql(),
        ));
    }

    let if_not_exists = if dialect.create_table_if_not_exists() {
        "IF NOT EXISTS "
    } else {
        ""
    };
    format!(
        "CREATE TABLE {if_not_exists}{table} (\n  {}\n);",
        cols.join(",\n  ")
    )
}

fn emit_indexes<D: SqlDialect>(dialect: &D, snapshot: &TableSnapshot) -> String {
    let table = dialect.quote_identifier(&snapshot.table);
    let mut sql = String::new();
    for identity in &snapshot.identities {
        let id_name = dialect.quote_identifier(&identity.name);
        let key_cols = identity
            .columns
            .iter()
            .map(|k| dialect.quote_identifier(k))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str("\n\n");
        sql.push_str(&format_create_index(
            true,
            true,
            &id_name,
            &table,
            &key_cols,
            identity.predicate.as_deref(),
            dialect.name(),
            identity.nils_distinct,
            None,
        ));
        sql.push(';');
    }
    for index in &snapshot.indexes {
        let idx_name = dialect.quote_identifier(&index.name);
        let key_cols = index
            .columns
            .iter()
            .map(|k| dialect.quote_identifier(k))
            .collect::<Vec<_>>()
            .join(", ");
        sql.push_str("\n\n");
        sql.push_str(&format_create_index(
            false,
            true,
            &idx_name,
            &table,
            &key_cols,
            index.predicate.as_deref(),
            dialect.name(),
            true,
            index.method.as_deref(),
        ));
        sql.push(';');
    }
    sql
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn format_create_index(
    unique: bool,
    if_not_exists: bool,
    name: &str,
    table: &str,
    columns: &str,
    predicate: Option<&str>,
    dialect: &str,
    nils_distinct: bool,
    method: Option<&str>,
) -> String {
    let unique_sql = if unique { "UNIQUE " } else { "" };
    let exists_sql = if if_not_exists { "IF NOT EXISTS " } else { "" };
    let using_sql = match method {
        Some(method) if dialect == "postgres" && !method.eq_ignore_ascii_case("btree") => {
            format!(" USING {method}")
        }
        _ => String::new(),
    };
    let nulls_sql = if unique && !nils_distinct && dialect == "postgres" {
        " NULLS NOT DISTINCT"
    } else {
        ""
    };
    let where_sql = predicate
        .filter(|predicate| !predicate.is_empty())
        .map(|predicate| format!(" WHERE {predicate}"))
        .unwrap_or_default();
    format!(
        "CREATE {unique_sql}INDEX {exists_sql}{name} ON {table}{using_sql} ({columns}){nulls_sql}{where_sql}"
    )
}

fn table_of(op: &SchemaOperation) -> Option<&str> {
    match op {
        SchemaOperation::CreateTable(snapshot) => Some(snapshot.table.as_str()),
        SchemaOperation::DropTable(name) => Some(name.as_str()),
        SchemaOperation::RenameTable { new_name, .. } => Some(new_name.as_str()),
        SchemaOperation::AddColumn { table, .. }
        | SchemaOperation::DropColumn { table, .. }
        | SchemaOperation::RenameColumn { table, .. }
        | SchemaOperation::AlterColumnType { table, .. }
        | SchemaOperation::SetNullable { table, .. }
        | SchemaOperation::SetDefault { table, .. }
        | SchemaOperation::CreateIdentity { table, .. }
        | SchemaOperation::DropIdentity { table, .. }
        | SchemaOperation::CreateIndex { table, .. }
        | SchemaOperation::DropIndex { table, .. }
        | SchemaOperation::AddCheck { table, .. }
        | SchemaOperation::DropCheck { table, .. }
        | SchemaOperation::AddReference { table, .. }
        | SchemaOperation::DropReference { table, .. } => Some(table.as_str()),
    }
}

fn needs_sqlite_rebuild(op: &SchemaOperation) -> bool {
    matches!(
        op,
        SchemaOperation::AlterColumnType { .. }
            | SchemaOperation::SetNullable { .. }
            | SchemaOperation::SetDefault { .. }
            | SchemaOperation::AddCheck { .. }
            | SchemaOperation::DropCheck { .. }
            | SchemaOperation::AddReference { .. }
            | SchemaOperation::DropReference { .. }
    )
}

pub fn emit_sql<D: SqlDialect>(
    dialect: &D,
    operations: &[SchemaOperation],
    previous: &[TableSnapshot],
    targets: &[TableSnapshot],
) -> String {
    let rebuild: HashSet<String> = if dialect.name() == "sqlite" {
        operations
            .iter()
            .filter(|op| needs_sqlite_rebuild(op))
            .filter_map(table_of)
            .map(str::to_string)
            .collect()
    } else {
        HashSet::new()
    };

    let mut stmts = Vec::new();
    let mut rebuilt = HashSet::new();

    for op in operations {
        if matches!(op, SchemaOperation::RenameTable { .. }) {
            let (up, _) = generate_operation_sql(dialect, op);
            if !up.is_empty() {
                stmts.push(up);
            }
            continue;
        }
        let table = table_of(op).unwrap_or_default();
        if rebuild.contains(table)
            && !matches!(
                op,
                SchemaOperation::CreateTable(_) | SchemaOperation::DropTable(_)
            )
        {
            if matches!(op, SchemaOperation::RenameColumn { .. }) {
                let (up, _) = generate_operation_sql(dialect, op);
                if !up.is_empty() {
                    stmts.push(up);
                }
            }
            if rebuilt.insert(table.to_string())
                && let (Some(old), Some(new)) = (
                    previous.iter().find(|s| s.table == table),
                    targets.iter().find(|s| s.table == table),
                )
            {
                stmts.push(emit_sqlite_rebuild(dialect, old, new, previous, operations));
            }
            continue;
        }
        let (up, _) = generate_operation_sql(dialect, op);
        if !up.is_empty() {
            stmts.push(up);
        }
    }

    stmts.join("\n\n")
}

fn emit_sqlite_rebuild<D: SqlDialect>(
    dialect: &D,
    old: &TableSnapshot,
    new: &TableSnapshot,
    all: &[TableSnapshot],
    operations: &[SchemaOperation],
) -> String {
    let live_names = {
        let mut names: HashSet<String> = old.columns.iter().map(|c| c.name.clone()).collect();
        for op in operations {
            if let SchemaOperation::RenameColumn {
                table,
                old_name,
                new_name,
            } = op
                && table == &new.table
            {
                names.remove(old_name);
                names.insert(new_name.clone());
            }
        }
        names
    };

    let children: Vec<&TableSnapshot> = all
        .iter()
        .filter(|table| {
            table.table != new.table
                && table
                    .references
                    .iter()
                    .any(|reference| reference.target_table == new.table)
        })
        .collect();

    let mut sql = String::new();
    for child in &children {
        let hold = dialect.quote_identifier(&format!("{}__ash_hold", child.table));
        let src = dialect.quote_identifier(&child.table);
        sql.push_str(&format!("CREATE TABLE {hold} AS SELECT * FROM {src};\n\n"));
    }

    let mut copy = new.clone();
    copy.table = format!("{}__ash_new", new.table);
    sql.push_str(&emit_create_table(dialect, &copy));
    let dest = dialect.quote_identifier(&copy.table);
    let src = dialect.quote_identifier(&new.table);
    let cols: Vec<String> = new
        .columns
        .iter()
        .filter(|c| live_names.contains(&c.name))
        .map(|c| dialect.quote_identifier(&c.name))
        .collect();
    if !cols.is_empty() {
        let list = cols.join(", ");
        sql.push_str(&format!(
            "\n\nINSERT INTO {dest} ({list})\nSELECT {list} FROM {src};"
        ));
    }
    sql.push_str(&format!("\n\nDROP TABLE {src};"));
    sql.push_str(&format!("\n\nALTER TABLE {dest} RENAME TO {src};"));
    let mut indexed = new.clone();
    indexed.table = new.table.clone();
    sql.push_str(&emit_indexes(dialect, &indexed));
    for child in &children {
        let hold = dialect.quote_identifier(&format!("{}__ash_hold", child.table));
        let src = dialect.quote_identifier(&child.table);
        let child_cols: Vec<String> = child
            .columns
            .iter()
            .map(|c| dialect.quote_identifier(&c.name))
            .collect();
        let list = child_cols.join(", ");
        sql.push_str(&format!("\n\nDELETE FROM {src};"));
        sql.push_str(&format!(
            "\n\nINSERT INTO {src} ({list}) SELECT {list} FROM {hold};"
        ));
        sql.push_str(&format!("\n\nDROP TABLE {hold};"));
    }
    sql
}
