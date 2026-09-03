use clap::{Parser, Subcommand};

use cargo_ash::{
    run_dump, run_generate, run_migrate, run_rollback, run_status, DumpArgs, GenerateArgs,
    MigrateArgs, RollbackArgs, StatusArgs,
};

#[derive(Parser, Debug)]
#[command(
    name = "cargo-ash",
    bin_name = "cargo ash",
    version,
    about = "Declarative schema migrations and tooling for ash-rust"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage declarative migrations
    Migrations {
        #[command(subcommand)]
        command: MigrationCommands,
    },
    /// Manage database schema snapshots
    Schema {
        #[command(subcommand)]
        command: SchemaCommands,
    },
    /// Run pending migrations (shortcut for `migrations run`)
    Migrate(MigrateArgs),
    /// Rollback migrations (shortcut for `migrations rollback`)
    Rollback(RollbackArgs),
    /// Generate a new migration (shortcut for `migrations generate`)
    Generate(GenerateArgs),
    /// View migration status (shortcut for `migrations status`)
    Status(StatusArgs),
    /// Dump database schema (shortcut for `schema dump`)
    Dump(DumpArgs),
}

#[derive(Subcommand, Debug)]
pub enum MigrationCommands {
    /// Generate a new migration from snapshots or changes
    Generate(GenerateArgs),
    /// Run pending migrations
    Run(MigrateArgs),
    /// Rollback applied migrations
    Rollback(RollbackArgs),
    /// View status of migrations
    Status(StatusArgs),
}

#[derive(Subcommand, Debug)]
pub enum SchemaCommands {
    /// Dump database schema to snapshot JSON files
    Dump(DumpArgs),
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args: Vec<String> = std::env::args().collect();
    // When invoked as `cargo ash ...`, cargo passes `ash` as args[1]
    if args.get(1).map(|s| s.as_str()) == Some("ash") {
        args.remove(1);
    }

    let cli = Cli::parse_from(args);

    match cli.command {
        Commands::Migrations { command } => match command {
            MigrationCommands::Generate(args) => run_generate(args)?,
            MigrationCommands::Run(args) => run_migrate(args).await?,
            MigrationCommands::Rollback(args) => run_rollback(args).await?,
            MigrationCommands::Status(args) => run_status(args).await?,
        },
        Commands::Schema { command } => match command {
            SchemaCommands::Dump(args) => run_dump(args).await?,
        },
        Commands::Migrate(args) => run_migrate(args).await?,
        Commands::Rollback(args) => run_rollback(args).await?,
        Commands::Generate(args) => run_generate(args)?,
        Commands::Status(args) => run_status(args).await?,
        Commands::Dump(args) => run_dump(args).await?,
    }

    Ok(())
}
