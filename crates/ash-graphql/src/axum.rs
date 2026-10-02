//! Axum web framework integration helpers for `ash-graphql`.

use std::sync::Arc;
use async_graphql::dynamic::Schema;
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLResponse};
use axum::{
    extract::{State, WebSocketUpgrade},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};

pub use crate::ws::{Hub as SubscriptionHub, SHARED_BUFFER};

/// Default GraphQL execution handler for Axum.
pub async fn graphql_handler(
    State(schema): State<Arc<Schema>>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

/// Handler serving the interactive GraphiQL IDE.
pub async fn graphiql_handler() -> impl IntoResponse {
    Html(
        GraphiQLSource::build()
            .endpoint("/graphql")
            .subscription_endpoint("/graphql/ws")
            .finish(),
    )
}

/// Creates a standard Axum router configured with GraphQL, GraphiQL and subscription
/// endpoints.
///
/// Endpoints:
/// - `POST /graphql`: GraphQL query and mutation endpoint
/// - `GET /graphql` (or `/graphiql`): GraphiQL playground
/// - `GET /graphql/ws`: subscriptions over WebSocket, speaking both the
///   `graphql-transport-ws` and the older `graphql-ws` protocols. Over
///   `graphql-transport-ws`, subscribers to the same subscription share one execution:
///   each event is resolved and serialized once for all of them, as Absinthe
///   deduplicates AshGraphql's subscriptions.
pub fn graphql_router(schema: Schema) -> Router {
    graphql_router_with_hub(schema, Arc::new(SubscriptionHub::default()))
}

/// [`graphql_router`], with the [`SubscriptionHub`] its shared subscriptions run in, to
/// see how many are running.
pub fn graphql_router_with_hub(schema: Schema, hub: Arc<SubscriptionHub>) -> Router {
    let shared_schema = Arc::new(schema.clone());
    let subscriptions = move |protocol: GraphQLProtocol, headers: HeaderMap, upgrade: WebSocketUpgrade| {
        let (schema, hub) = (schema.clone(), Arc::clone(&hub));
        async move { crate::ws::upgrade(protocol, headers, upgrade, schema, hub) }
    };

    Router::new()
        .route("/graphql/ws", get(subscriptions))
        .route("/graphql", post(graphql_handler))
        .route("/graphql", get(graphiql_handler))
        .route("/graphiql", get(graphiql_handler))
        .with_state(shared_schema)
}
