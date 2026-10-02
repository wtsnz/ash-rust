//! Axum web framework integration helpers for `ash-graphql`.

use std::sync::Arc;
use async_graphql::dynamic::Schema;
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use axum::{
    extract::State,
    response::{Html, IntoResponse},
    routing::{get, post},
    Router,
};

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
///   `graphql-transport-ws` and the older `graphql-ws` protocols
pub fn graphql_router(schema: Schema) -> Router {
    let subscriptions = GraphQLSubscription::new(schema.clone());
    let shared_schema = Arc::new(schema);

    Router::new()
        .route_service("/graphql/ws", subscriptions)
        .route("/graphql", post(graphql_handler))
        .route("/graphql", get(graphiql_handler))
        .route("/graphiql", get(graphiql_handler))
        .with_state(shared_schema)
}
