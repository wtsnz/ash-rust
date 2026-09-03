use std::fs;
use std::path::PathBuf;
use ash_sql::{
    diff_snapshots, generate_migration, generate_migration_version, PostgresDialect,
    SqliteDialect, TableSnapshot,
};
use clap::Args;

#[derive(Args, Debug)]
pub struct GenerateArgs {
    /// Name of the migration (e.g. add_priority_to_tickets)
    #[arg(short, long)]
    pub name: String,

    /// Target SQL dialect: "sqlite" or "postgres"
    #[arg(long, default_value = "sqlite")]
    pub dialect: String,

    /// Directory where migration files will be saved
    #[arg(long, default_value = "migrations")]
    pub dir: PathBuf,

    /// Directory containing current/previous resource snapshots (.json)
    #[arg(long, default_value = "snapshots")]
    pub snapshots_dir: PathBuf,

    /// Optional path to previous snapshot file
    #[arg(long)]
    pub from: Option<PathBuf>,

    /// Optional path to current snapshot file
    #[arg(long)]
    pub to: Option<PathBuf>,

    /// Dry run: print SQL without writing files
    #[arg(long)]
    pub dry_run: bool,
}

pub fn run(args: GenerateArgs) -> Result<(), Box<dyn std::error::Error>> {
    let version = generate_migration_version();
    let old_snapshot = match &args.from {
        Some(path) => {
            let data = fs::read_to_string(path)?;
            Some(serde_json::from_str::<TableSnapshot>(&data)?)
        }
        None => None,
    };

    let new_snapshot = match &args.to {
        Some(path) => {
            let data = fs::read_to_string(path)?;
            Some(serde_json::from_str::<TableSnapshot>(&data)?)
        }
        None => None,
    };

    let ops = diff_snapshots(old_snapshot.as_ref(), new_snapshot.as_ref());

    let (up_sql, down_sql) = match args.dialect.to_lowercase().as_str() {
        "postgres" | "postgresql" => {
            let files = generate_migration(&PostgresDialect, &version, &ops);
            (files.up_sql, files.down_sql)
        }
        _ => {
            let files = generate_migration(&SqliteDialect, &version, &ops);
            (files.up_sql, files.down_sql)
        }
    };

    if args.dry_run {
        println!("=== Migration: {}_{}.up.sql ===", version, args.name);
        println!("{up_sql}");
        println!("=== Migration: {}_{}.down.sql ===", version, args.name);
        println!("{down_sql}");
        return Ok(());
    }

    fs::create_dir_all(&args.dir)?;
    let up_file = args.dir.join(format!("{}_{}.up.sql", version, args.name));
    let down_file = args.dir.join(format!("{}_{}.down.sql", version, args.name));

    fs::write(&up_file, up_sql)?;
    fs::write(&down_file, down_sql)?;

    println!("Generated migration files:");
    println!("  {}", up_file.display());
    println!("  {}", down_file.display());

    Ok(())
}
