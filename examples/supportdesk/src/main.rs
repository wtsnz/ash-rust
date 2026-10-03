//! `supportdesk`: serves the desk on Postgres.
//!
//! ```bash
//! cargo run --release -p supportdesk --bin fixture -- --out fixture.json
//! DATABASE_URL=postgres://… FIXTURE=fixture.json cargo run --release -p supportdesk
//! ```
//!
//! With `FIXTURE`, the desk's tables are emptied and the fixture loaded before serving.

use std::time::Instant;

use ash_pubsub::{ContextPubSubExt, PubSub};
use supportdesk::server::router;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::var("DATABASE_URL").map_err(|_| "DATABASE_URL is required")?;
    let port: u16 = std::env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(4000);
    let db = ash_postgres::Postgres::connect(&url).await?;
    let resources: Vec<&ash_core::ResourceDef> = supportdesk::DESK_DEF.resources.to_vec();
    db.install(&resources).await?;

    let pubsub = PubSub::new();
    let ctx = ash_core::Context::new(db.clone()).with_pubsub(std::sync::Arc::new(pubsub.clone()));
    let mut seeded_ms = 0;
    if let Ok(path) = std::env::var("FIXTURE") {
        let started = Instant::now();
        let tables: Vec<String> = resources.iter().map(|r| format!("\"{}\"", r.table)).collect();
        let pool = db.pool().expect("a pool, outside any transaction");
        sqlx::query(&format!("TRUNCATE {} CASCADE", tables.join(", "))).execute(pool).await?;
        let fixture: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        supportdesk::fixture::load(&ctx, &fixture).await?;
        seeded_ms = started.elapsed().as_millis() as u64;
        println!("  Loaded {path} in {seeded_ms} ms");
    }

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    println!("  Supportdesk on ash-rust · GraphQL http://{addr}/graphql · ws://{addr}/graphql/ws");
    axum::serve(listener, router(ctx, pubsub, seeded_ms)?).await?;
    Ok(())
}
