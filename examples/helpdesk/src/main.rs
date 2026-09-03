use clap::Parser;
use helpdesk::cli::{Cli, run};

#[tokio::main]
async fn main() {
    if let Err(err) = run(Cli::parse()).await {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
