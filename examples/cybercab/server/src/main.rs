//! `cybercab`: run the command center's API with a simulated fleet, or write its SDK.
//!
//! ```bash
//! cargo run -p cybercab                   # API on :4000, fleet simulated at 8x
//! cargo run -p cybercab -- --codegen-only # write frontend/src/lib/ash.ts and exit
//! SIM_SPEED=4 DEMAND=1.5 PORT=4100 cargo run -p cybercab
//! FLEET=5000 cargo run -p cybercab --release  # a bigger fleet, and a busier city
//! DATABASE_URL=postgres://… cargo run -p cybercab  # on Postgres; empties its own tables
//! ```

use std::path::Path;

use ash_core::DataLayer;
use ash_memory::Memory;
use ash_pubsub::PubSub;
use cybercab::city::City;
use cybercab::server::{context, router, typescript_sdk};
use cybercab::sim::{SimConfig, Simulation};

fn env<T: std::str::FromStr>(name: &str, default: T) -> T {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let sdk_path = manifest.join("frontend/src/lib/ash.ts");
    std::fs::create_dir_all(sdk_path.parent().expect("a parent"))?;
    std::fs::write(&sdk_path, typescript_sdk()?)?;
    println!("  Generated TypeScript SDK -> {}", sdk_path.display());
    if std::env::args().any(|arg| arg == "--codegen-only") {
        return Ok(());
    }

    match std::env::var("DATABASE_URL") {
        Ok(url) => serve(cybercab::server::postgres(&url).await?, "Postgres").await,
        Err(_) => serve(Memory::new(), "memory").await,
    }
}

/// Seeds the city on `data`, sets the fleet running, and serves the API.
async fn serve<D>(data: D, store: &str) -> Result<(), Box<dyn std::error::Error>>
where
    D: DataLayer + Clone + Send + Sync + 'static,
{
    let config = SimConfig {
        speedup: env("SIM_SPEED", 8.0),
        demand: env("DEMAND", 1.0),
        seed: env("SEED", 0xCAB5),
    };
    let pubsub = PubSub::new();
    let ctx = context(data, &pubsub);
    let city = City::austin();
    let fleet = env("FLEET", cybercab::seed::FLEET_SIZE);
    cybercab::seed::austin_with_fleet(&ctx, &city, config.seed, fleet).await?;
    let simulation = Simulation::new(ctx.clone(), city, config.clone()).await?;
    tokio::spawn(simulation.run());

    let port: u16 = env("PORT", 4000);
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!(
        "\n  Cybercab Command Center API · Austin · {fleet} cabs at {}x, on {store}",
        config.speedup
    );
    println!("  GraphQL      http://{addr}/graphql");
    println!("  GraphiQL     http://{addr}/graphiql");
    println!("  Live (ws)    ws://{addr}/graphql/ws\n");
    axum::serve(listener, router(ctx, pubsub)?).await?;
    Ok(())
}
