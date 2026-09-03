use clap::Parser;
use kanban::cli::{Cli, run};

#[tokio::main]
async fn main() {
    if let Err(err) = run(Cli::parse()).await {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
