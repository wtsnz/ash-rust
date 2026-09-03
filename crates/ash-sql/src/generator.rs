use crate::dialect::SqlDialect;
use crate::diff::SchemaOperation;

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

            let mut up = format!("CREATE TABLE IF NOT EXISTS {table} (\n  {}\n);", cols.join(",\n  "));

            for id in &snapshot.identities {
                let id_name = dialect.quote_identifier(&id.name);
                let key_cols = id
                    .columns
                    .iter()
                    .map(|k| dialect.quote_identifier(k))
                    .collect::<Vec<_>>()
                    .join(", ");
                up.push_str(&format!(
                    "\n\nCREATE UNIQUE INDEX IF NOT EXISTS {id_name} ON {table} ({key_cols});"
                ));
            }

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
                let up = format!("ALTER TABLE {t} ALTER COLUMN {col} TYPE {new_type};");
                let down = format!("ALTER TABLE {t} ALTER COLUMN {col} TYPE {old_type};");
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
                let up = "-- SQLite does not support modifying column nullability directly".to_string();
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
            let up = format!("CREATE UNIQUE INDEX IF NOT EXISTS {id_name} ON {t} ({key_cols});");
            let down = format!("DROP INDEX IF EXISTS {id_name};");
            (up, down)
        }
        SchemaOperation::DropIdentity { table: _, name } => {
            let id_name = dialect.quote_identifier(name);
            let up = format!("DROP INDEX IF EXISTS {id_name};");
            let down = format!("-- Rollback DROP INDEX {id_name}");
            (up, down)
        }
        SchemaOperation::AddReference { table, reference } => {
            let t = dialect.quote_identifier(table);
            let ref_name = dialect.quote_identifier(&reference.name);
            let col = dialect.quote_identifier(&reference.column);
            let target_t = dialect.quote_identifier(&reference.target_table);
            let target_col = dialect.quote_identifier(&reference.target_column);
            let on_del = &reference.on_delete;

            let up = format!(
                "ALTER TABLE {t} ADD CONSTRAINT {ref_name} FOREIGN KEY ({col}) REFERENCES {target_t} ({target_col}) ON DELETE {on_del};"
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
