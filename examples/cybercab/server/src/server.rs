//! The control room's API: GraphQL queries, mutations and live subscriptions over the
//! four domains, and the TypeScript SDK the frontend is generated from.

use std::sync::Arc;

use ash_core::{Context, DataLayer, ResourceDef};
use ash_graphql::AshGraphQL;
use ash_pubsub::PubSub;
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use axum::Router;
use axum::routing::get;
use tower_http::cors::CorsLayer;

use crate::DOMAINS;

/// Every resource the API serves.
pub fn resources() -> Vec<&'static ResourceDef> {
    DOMAINS
        .iter()
        .flat_map(|domain| domain.resources.iter().copied())
        .collect()
}

/// The GraphQL API over `ctx`, with subscriptions fed by `pubsub`. Changes reach
/// subscribers because `ctx` carries a `PubSubNotifier` on the same `PubSub`.
pub fn router<D: DataLayer + Clone + Send + Sync + 'static>(
    ctx: Context<D>,
    pubsub: PubSub,
) -> Result<Router, async_graphql::dynamic::SchemaError> {
    let schema = AshGraphQL::from_resources(&resources())
        .with_pubsub(pubsub)
        .with_dataloader()
        .finish_with_context(ctx)?;
    Ok(ash_graphql::axum::graphql_router(schema)
        .route("/health", get(|| async { "OK" }))
        .layer(CorsLayer::permissive()))
}

/// `GET /metrics`: the fleet's size and the simulation's ticks, as both Cybercab servers
/// report them for the benchmark. `metrics` is `None` when the simulation isn't running.
pub fn metrics_router(metrics: Option<crate::sim::TickMetrics>, fleet: usize) -> Router {
    Router::new().route(
        "/metrics",
        get(move || {
            let metrics = metrics.clone();
            async move {
                let simulating = metrics.is_some();
                let mut body = metrics.map(|m| m.to_json()).unwrap_or_else(|| {
                    serde_json::json!({ "ticks": 0, "errors": 0, "recent_tick_ms": [] })
                });
                body["fleet"] = fleet.into();
                body["simulating"] = simulating.into();
                axum::Json(body)
            }
        }),
    )
}

/// The frontend's SDK: types, Zod schemas, the client with live queries, and React hooks.
pub fn typescript_sdk() -> Result<String, ash_typescript::CodegenError> {
    let mut generator = TypeScriptGenerator::new().config(
        TypeScriptConfig::new()
            .with_zod(true)
            .with_client(true)
            .with_react(true)
            .with_subscriptions(true)
            .with_client_name("CommandClient"),
    );
    for domain in DOMAINS {
        generator = generator.add_domain(domain);
    }
    generator.generate_consolidated()
}

/// A notifier-carrying context for the simulation and the API to share.
pub fn context<D>(data: D, pubsub: &PubSub) -> Context<D> {
    use ash_pubsub::ContextPubSubExt;
    Context::new(data).with_pubsub(Arc::new(pubsub.clone()))
}

/// Connects to Postgres at `url`, installs the command center's tables, and empties them,
/// so a shift starts from the seed. It touches only its own tables.
pub async fn postgres(url: &str) -> ash_core::Result<ash_postgres::Postgres> {
    let db = ash_postgres::Postgres::connect(url).await?;
    db.install(&resources()).await?;
    let tables: Vec<String> = resources()
        .iter()
        .map(|resource| format!("\"{}\"", resource.table))
        .collect();
    db.execute_sql(&format!("TRUNCATE {} CASCADE", tables.join(", "))).await?;
    Ok(db)
}
