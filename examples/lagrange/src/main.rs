//! `lagrange`: simulate a week in the Sol system, serve the API, or write the SDK.
//!
//! ```bash
//! cargo run -p lagrange -- simulate
//! cargo run -p lagrange -- simulate --sqlite target/lagrange/fleet.db
//! cargo run -p lagrange -- serve --port 4000
//! cargo run -p lagrange -- sdk --out examples/lagrange/sdk/lagrange.ts
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use ash_core::{DataLayer, TransactionSupport};
use ash_mailer::ConsoleMailer;
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use clap::{Args, Parser, Subcommand};
use lagrange::{Lagrange, Planet};

#[derive(Parser)]
#[command(name = "lagrange", about = "Interplanetary freight and docking on ash-rust")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs a week of bookings, voyages and deliveries on a fresh database.
    Simulate(Database),
    /// Seeds the Sol system if the database is empty, then serves the API.
    Serve {
        #[command(flatten)]
        database: Database,
        #[arg(long, default_value_t = 4000)]
        port: u16,
        /// Secret that signs session tokens.
        #[arg(long, env = "LAGRANGE_JWT_SECRET", default_value = "lagrange-development-signing-key")]
        jwt_secret: String,
    },
    /// Writes the TypeScript SDK and Zod schemas.
    Sdk {
        #[arg(long, default_value = "sdk/lagrange.ts")]
        out: PathBuf,
    },
}

/// The fleet database: in memory by default, or a SQLite file, or Postgres.
#[derive(Args)]
struct Database {
    #[arg(long, conflicts_with = "postgres")]
    sqlite: Option<PathBuf>,
    #[arg(long)]
    postgres: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

type Failure = Box<dyn std::error::Error>;

async fn run(cli: Cli) -> Result<(), Failure> {
    match cli.command {
        Command::Sdk { out } => {
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&out, lagrange::server::typescript_sdk(true)?)?;
            println!("wrote {}", out.display());
            Ok(())
        }
        Command::Simulate(database) => with_database(database, Simulate).await,
        Command::Serve {
            database,
            port,
            jwt_secret,
        } => with_database(database, Serve { port, jwt_secret }).await,
    }
}

/// Something to do with a platform, whichever database it runs on.
trait Task {
    async fn run<D>(self, app: Lagrange<D>) -> Result<(), Failure>
    where
        D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static;
}

async fn telemetry(next_to: Option<&Path>) -> Result<Sqlite, Failure> {
    let telemetry = match next_to {
        Some(fleet) => Sqlite::file(fleet.with_file_name("telemetry.db")).await?,
        None => Sqlite::memory().await?,
    };
    lagrange::Telemetry::new(telemetry.clone()).install().await?;
    Ok(telemetry)
}

async fn with_database(database: Database, task: impl Task) -> Result<(), Failure> {
    let mailer = ConsoleMailer::new().with_tag("lagrange");
    if let Some(path) = database.sqlite {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let fleet = Sqlite::file(&path).await?;
        fleet.migrate(lagrange::migrations_dir()).await?;
        task.run(Lagrange::new(fleet, telemetry(Some(&path)).await?, mailer)).await
    } else if let Some(url) = database.postgres {
        let fleet = Postgres::connect(&url).await?;
        fleet.migrate(lagrange::migrations_dir()).await?;
        task.run(Lagrange::new(fleet, telemetry(None).await?, mailer)).await
    } else {
        task.run(Lagrange::new(Memory::new(), telemetry(None).await?, mailer)).await
    }
}

struct Simulate;

impl Task for Simulate {
    async fn run<D>(self, app: Lagrange<D>) -> Result<(), Failure>
    where
        D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
    {
        let (_, log) = lagrange::sim::week_in_sol(&app).await?;
        print!("{log}");
        Ok(())
    }
}

struct Serve {
    port: u16,
    jwt_secret: String,
}

impl Task for Serve {
    async fn run<D>(self, app: Lagrange<D>) -> Result<(), Failure>
    where
        D: DataLayer + TransactionSupport + Clone + Send + Sync + 'static,
    {
        if Planet::query(&app.anonymous()).count().await? == 0 {
            lagrange::seed::sol(&app).await?;
            println!("seeded the Sol system; sign in as ada@helios-freight.example / {}", lagrange::seed::PASSWORD);
        }
        let jwt = Arc::new(ash_authentication::JwtService::new(self.jwt_secret));
        let router = lagrange::server::router(Arc::new(app), jwt)?;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", self.port)).await?;
        println!("GraphQL on http://127.0.0.1:{}/graphql", self.port);
        axum::serve(listener, router).await?;
        Ok(())
    }
}
