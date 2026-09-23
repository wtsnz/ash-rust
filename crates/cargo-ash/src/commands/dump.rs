use std::fs;
use std::path::PathBuf;
use ash_sql::{ColumnSnapshot, TableSnapshot};
use clap::Args;
use sqlx::Row;

#[derive(Args, Debug)]
pub struct DumpArgs {
    /// Database connection URL (e.g. postgres://... or sqlite://...)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Output directory for snapshot files
    #[arg(long, default_value = "snapshots")]
    pub output: PathBuf,
}

pub async fn run(args: DumpArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args
        .database_url
        .unwrap_or_else(|| "sqlite://ash.db".to_string());

    let is_postgres = url.starts_with("postgres://") || url.starts_with("postgresql://");
    fs::create_dir_all(&args.output)?;

    let snapshots = if is_postgres {
        dump_postgres(&url).await?
    } else {
        dump_sqlite(&url).await?
    };

    let count = snapshots.len();
    for snapshot in snapshots {
        let file_path = args.output.join(format!("{}.json", snapshot.table));
        let json = serde_json::to_string_pretty(&snapshot)?;
        fs::write(&file_path, json)?;
        println!("  Dumped table snapshot: {}", file_path.display());
    }

    println!("Successfully dumped {} table snapshot(s) to `{}`.", count, args.output.display());
    Ok(())
}

async fn dump_sqlite(url: &str) -> Result<Vec<TableSnapshot>, Box<dyn std::error::Error>> {
    let db = ash_sqlite::Sqlite::connect(url).await?;
    let pool = db.pool().ok_or("Cannot acquire SQLite connection pool")?;

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name != '_ash_schema_migrations' ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut snapshots = Vec::new();
    for table in tables {
        let rows = sqlx::query(&format!("PRAGMA table_info(\"{table}\")"))
            .fetch_all(pool)
            .await?;

        let mut columns = Vec::new();
        let mut primary_key = Vec::new();

        for r in rows {
            let name: String = r.get("name");
            let col_type: String = r.get("type");
            let notnull: i64 = r.get("notnull");
            let pk: i64 = r.get("pk");

            let col_snap = ColumnSnapshot {
                name: name.clone(),
                sql_type: col_type,
                nullable: notnull == 0,
                default: None,
                is_primary_key: pk > 0,
            };
            if pk > 0 {
                primary_key.push(name.clone());
            }
            columns.push(col_snap);
        }

        snapshots.push(TableSnapshot {
            format_version: 1,
            table,
            schema: None,
            columns,
            primary_key,
            identities: Vec::new(),
            indexes: Vec::new(),
            references: Vec::new(),
        });
    }

    Ok(snapshots)
}

async fn dump_postgres(url: &str) -> Result<Vec<TableSnapshot>, Box<dyn std::error::Error>> {
    let db = ash_postgres::Postgres::connect(url).await?;
    let pool = db.pool().ok_or("Cannot acquire PostgreSQL connection pool")?;

    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name != '_ash_schema_migrations' ORDER BY table_name ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut snapshots = Vec::new();
    for table in tables {
        let rows = sqlx::query(
            "SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = 'public' AND table_name = $1 ORDER BY ordinal_position ASC"
        )
        .bind(&table)
        .fetch_all(pool)
        .await?;

        let mut columns = Vec::new();
        let primary_key = Vec::new();

        for r in rows {
            let name: String = r.get("column_name");
            let col_type: String = r.get("data_type");
            let is_nullable: String = r.get("is_nullable");
            let default: Option<String> = r.get("column_default");

            let col_snap = ColumnSnapshot {
                name,
                sql_type: col_type,
                nullable: is_nullable == "YES",
                default,
                is_primary_key: false,
            };
            columns.push(col_snap);
        }

        snapshots.push(TableSnapshot {
            format_version: 1,
            table,
            schema: Some("public".to_string()),
            columns,
            primary_key,
            identities: Vec::new(),
            indexes: Vec::new(),
            references: Vec::new(),
        });
    }

    Ok(snapshots)
}
