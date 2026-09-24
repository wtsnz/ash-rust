use clap::{Parser, Subcommand};

use cargo_ash::{
    DumpArgs, GenerateArgs, MigrateArgs, RollbackArgs, StatusArgs, TypeScriptArgs, run_dump,
    run_generate, run_migrate, run_reset, run_rollback, run_setup, run_status, run_ts,
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
    /// Generate client SDKs and code
    Codegen {
        #[command(subcommand)]
        command: CodegenCommands,
    },
    /// Run pending migrations (shortcut for `migrations run`)
    Migrate(MigrateArgs),
    /// Apply all pending migrations
    Setup(MigrateArgs),
    /// Roll back every applied migration, then run setup
    Reset(MigrateArgs),
    /// Rollback migrations (shortcut for `migrations rollback`)
    Rollback(RollbackArgs),
    /// Generate a new migration (shortcut for `migrations generate`)
    Generate(GenerateArgs),
    /// View migration status (shortcut for `migrations status`)
    Status(StatusArgs),
    /// Dump database schema (shortcut for `schema dump`)
    Dump(DumpArgs),
    /// Generate TypeScript SDK and Zod schemas (shortcut for `codegen ts`)
    Ts(TypeScriptArgs),
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

#[derive(Subcommand, Debug)]
pub enum CodegenCommands {
    /// Generate TypeScript definitions, Zod validation schemas, and isomorphic client SDK
    Ts(TypeScriptArgs),
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
        Commands::Codegen { command } => match command {
            CodegenCommands::Ts(args) => run_ts(args)?,
        },
        Commands::Migrate(args) => run_migrate(args).await?,
        Commands::Setup(args) => run_setup(args).await?,
        Commands::Reset(args) => run_reset(args).await?,
        Commands::Rollback(args) => run_rollback(args).await?,
        Commands::Generate(args) => run_generate(args)?,
        Commands::Status(args) => run_status(args).await?,
        Commands::Dump(args) => run_dump(args).await?,
        Commands::Ts(args) => run_ts(args)?,
    }

    Ok(())
}
