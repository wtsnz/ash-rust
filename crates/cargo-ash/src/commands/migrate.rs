use std::path::PathBuf;
use clap::Args;

#[derive(Args, Debug)]
pub struct MigrateArgs {
    /// Database connection URL (e.g. postgres://... or sqlite://...)
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Directory containing migrations
    #[arg(long, default_value = "migrations")]
    pub dir: PathBuf,
}

pub async fn run(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args
        .database_url
        .unwrap_or_else(|| "sqlite://ash.db".to_string());

    println!("Running migrations from `{}`...", args.dir.display());

    let applied = if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let db = ash_postgres::Postgres::connect(&url).await?;
        db.migrate(&args.dir).await?
    } else {
        let db = ash_sqlite::Sqlite::connect(&url).await?;
        db.migrate(&args.dir).await?
    };

    if applied.is_empty() {
        println!("Database schema is up to date (no pending migrations).");
    } else {
        for v in &applied {
            println!("  Applied migration: {}", v);
        }
        println!("Successfully applied {} migration(s).", applied.len());
    }

    Ok(())
}
