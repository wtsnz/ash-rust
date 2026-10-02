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
