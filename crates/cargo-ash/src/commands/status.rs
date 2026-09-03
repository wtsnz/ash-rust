use std::collections::HashSet;
use std::path::PathBuf;
use ash_sql::{MigrationExecutor, Migrator, PostgresDialect, SqliteDialect};
use clap::Args;

#[derive(Args, Debug)]
pub struct StatusArgs {
    /// Database connection URL (e.g. postgres://... or sqlite://...)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Directory containing migrations
    #[arg(long, default_value = "migrations")]
    pub dir: PathBuf,
}

pub async fn run(args: StatusArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args
        .database_url
        .unwrap_or_else(|| "sqlite://ash.db".to_string());

    let is_postgres = url.starts_with("postgres://") || url.starts_with("postgresql://");

    let (all_migrations, applied_set) = if is_postgres {
        let db = ash_postgres::Postgres::connect(&url).await?;
        let migrator = Migrator::new(PostgresDialect, &args.dir);
        let all = migrator.discover_migrations()?;
        let applied = db.applied_versions().await?;
        (all, applied.into_iter().collect::<HashSet<_>>())
    } else {
        let db = ash_sqlite::Sqlite::connect(&url).await?;
        let migrator = Migrator::new(SqliteDialect, &args.dir);
        let all = migrator.discover_migrations()?;
        let applied = db.applied_versions().await?;
        (all, applied.into_iter().collect::<HashSet<_>>())
    };

    println!("{:<20} {:<35} {:<10}", "Version", "Name", "Status");
    println!("{:-<20} {:-<35} {:-<10}", "", "", "");

    if all_migrations.is_empty() {
        println!("No migration files found in `{}`", args.dir.display());
        return Ok(());
    }

    for m in &all_migrations {
        let status = if applied_set.contains(&m.version) {
            "[Applied]"
        } else {
            "[Pending]"
        };
        println!("{:<20} {:<35} {:<10}", m.version, m.name, status);
    }

    Ok(())
}
