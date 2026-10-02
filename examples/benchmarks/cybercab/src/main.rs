//! `cybercab-bench`: the Rust and Elixir Cybercab servers, side by side, under the same
//! load. It starts each server fresh for every run, talks to it only over HTTP and
//! WebSocket, and writes what it measured to `results.jsonl` and `RESULTS.md`.
//!
//! ```bash
//! cybercab-bench run --rust-bin target/release/cybercab \
//!     --elixir-release examples/elixir/cybercab/_build/prod/rel/cybercab --out results
//! cybercab-bench report --out results
//! ```

mod gql;
mod record;
mod report;
mod scenarios;
mod server;
mod stats;

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde_json::json;

use crate::record::{Record, append};
use crate::scenarios::Plan;
use crate::server::{Kind, Launch};

#[derive(Parser)]
#[command(about = "Benchmarks the Rust and Elixir Cybercab servers against each other")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Runs the scenarios, then writes the report.
    Run {
        /// The Rust server's release binary.
        #[arg(long)]
        rust_bin: PathBuf,
        /// The Elixir server's release (`_build/prod/rel/cybercab`).
        #[arg(long)]
        elixir_release: PathBuf,
        /// Postgres, without a database: each server uses its own.
        #[arg(long, default_value = "postgres://postgres:postgres@127.0.0.1:55434")]
        postgres: String,
        #[arg(long, default_value = "results")]
        out: PathBuf,
        /// Which scenarios: reads, commands, sim, fanout, room.
        #[arg(long, value_delimiter = ',', default_value = "reads,commands,sim,fanout,room")]
        scenarios: Vec<String>,
        /// Which servers: rust, elixir-concurrent, elixir-upsert.
        #[arg(long, value_delimiter = ',', default_value = "rust,elixir-concurrent,elixir-upsert")]
        servers: Vec<String>,
        #[arg(long, default_value_t = 1)]
        reps: usize,
        /// Several fleet sizes, longer windows, more levels: much slower.
        #[arg(long)]
        full: bool,
        /// Only these reads (cab_by_id, fleet, trips_with_riders, aggregates, keyset_page,
        /// counted_page).
        #[arg(long, value_delimiter = ',')]
        queries: Vec<String>,
    },
    /// Compares one server across two runs: before and after a change.
    Compare {
        #[arg(long)]
        before: PathBuf,
        #[arg(long)]
        after: PathBuf,
        #[arg(long, default_value = "rust")]
        server: String,
    },
    /// Writes `RESULTS.md` from `results.jsonl`.
    Report {
        #[arg(long, default_value = "results")]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() {
    match Cli::parse().command {
        Command::Report { out } => report::write(&out),
        Command::Compare { before, after, server } => print!("{}", report::compare(&before, &after, &server)),
        Command::Run {
            rust_bin,
            elixir_release,
            postgres,
            out,
            scenarios,
            servers,
            reps,
            full,
            queries,
        } => {
            std::fs::create_dir_all(&out).expect("the output directory");
            let mut plan = if full { Plan::full() } else { Plan::standard() };
            plan.queries = queries;
            let kinds: Vec<Kind> = servers
                .iter()
                .map(|name| match name.as_str() {
                    "rust" => Kind::Rust { bin: rust_bin.clone() },
                    "elixir-concurrent" => Kind::Elixir { release: elixir_release.clone(), heartbeat: "concurrent" },
                    "elixir-upsert" => Kind::Elixir { release: elixir_release.clone(), heartbeat: "upsert" },
                    other => panic!("unknown server {other}"),
                })
                .collect();
            let wants = |name: &str| scenarios.iter().any(|s| s == name);

            for rep in 0..reps {
                println!("Repetition {}", rep + 1);
                for &fleet in &plan.fleets {
                    for kind in &kinds {
                        let launch = Launch { kind: kind.clone(), fleet, simulating: true, postgres: postgres.clone() };
                        let server = launch.start().await;
                        boot(&out, &server, fleet, rep);
                        tokio::time::sleep(plan.settle).await;
                        if wants("sim") {
                            append(&out, &scenarios::sim::run(&plan, &server, fleet, rep).await);
                        }
                        if wants("fanout") {
                            for record in scenarios::fanout::run(&plan, &server, fleet, rep).await {
                                append(&out, &record);
                            }
                        }
                        // Reads and commands don't depend on how the heartbeat is written,
                        // so the upsert heartbeat skips them unless it's the only Elixir.
                        let upsert = matches!(kind, Kind::Elixir { heartbeat: "upsert", .. });
                        let other_elixir = kinds.iter().any(|k| matches!(k, Kind::Elixir { heartbeat: "concurrent", .. }));
                        if !(upsert && other_elixir) {
                            if wants("reads") {
                                for record in scenarios::reads::run(&plan, &server, fleet, rep).await {
                                    append(&out, &record);
                                }
                            }
                            if wants("commands") {
                                for record in scenarios::commands::run(&plan, &server, fleet, rep).await {
                                    append(&out, &record);
                                }
                            }
                        }
                        if wants("room") {
                            append(&out, &scenarios::room::run(&plan, &server, fleet, rep).await);
                        }
                        server.stop().await;
                    }
                }
            }
            report::write(&out);
            println!("Wrote {}", out.join("RESULTS.md").display());
        }
    }
}

fn boot(out: &std::path::Path, server: &server::Running, fleet: usize, rep: usize) {
    append(
        out,
        &Record {
            scenario: "boot".into(),
            server: server.label.clone(),
            params: json!({ "fleet": fleet }),
            metrics: json!({ "boot_s": server.boot.as_secs_f64() }),
            rep,
        },
    );
}
