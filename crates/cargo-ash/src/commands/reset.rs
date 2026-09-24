use super::migrate::MigrateArgs;

pub async fn run(args: MigrateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let url = args
        .database_url
        .clone()
        .unwrap_or_else(|| "sqlite://ash.db".to_string());

    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        let db = ash_postgres::Postgres::connect(&url).await?;
        while let Some(version) = db.rollback(&args.dir).await? {
            println!("  Rolled back migration: {}", version);
        }
    } else {
        let db = ash_sqlite::Sqlite::connect(&url).await?;
        while let Some(version) = db.rollback(&args.dir).await? {
            println!("  Rolled back migration: {}", version);
        }
    }

    super::setup::run(args).await
}
