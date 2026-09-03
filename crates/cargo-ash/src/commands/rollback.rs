use std::path::PathBuf;
use ash_sql::{Migrator, PostgresDialect, SqliteDialect};
use clap::Args;

#[derive(Args, Debug)]
pub struct RollbackArgs {
    /// Database connection URL (e.g. postgres://... or sqlite://...)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Directory containing migrations
    #[arg(long, default_value = "migrations")]
    pub dir: PathBuf,

    /// Target migration version to rollback to
    #[arg(long)]
    pub to: Option<String>,
}

pub async fn run(args: RollbackArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args
        .database_url
        .unwrap_or_else(|| "sqlite://ash.db".to_string());

    let is_postgres = url.starts_with("postgres://") || url.starts_with("postgresql://");

    if is_postgres {
        let db = ash_postgres::Postgres::connect(&url).await?;
        let migrator = Migrator::new(PostgresDialect, &args.dir);
        if let Some(target) = &args.to {
            let rolled = migrator.rollback_to(&db, target).await?;
            if rolled.is_empty() {
                println!("No migrations were rolled back.");
            } else {
                for v in &rolled {
                    println!("  Rolled back migration: {}", v);
                }
                println!("Successfully rolled back {} migration(s).", rolled.len());
            }
        } else {
            match migrator.rollback(&db).await? {
                Some(v) => println!("Successfully rolled back migration: {}", v),
                None => println!("No applied migrations found to rollback."),
            }
        }
    } else {
        let db = ash_sqlite::Sqlite::connect(&url).await?;
        let migrator = Migrator::new(SqliteDialect, &args.dir);
        if let Some(target) = &args.to {
            let rolled = migrator.rollback_to(&db, target).await?;
            if rolled.is_empty() {
                println!("No migrations were rolled back.");
            } else {
                for v in &rolled {
                    println!("  Rolled back migration: {}", v);
                }
                println!("Successfully rolled back {} migration(s).", rolled.len());
            }
        } else {
            match migrator.rollback(&db).await? {
                Some(v) => println!("Successfully rolled back migration: {}", v),
                None => println!("No applied migrations found to rollback."),
            }
        }
    }

    Ok(())
}
