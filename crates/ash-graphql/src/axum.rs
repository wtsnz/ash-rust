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

/// Who a request runs as, from its headers: a query or mutation over HTTP, or each
/// subscription on a WebSocket, from its upgrade request's headers. As AshGraphql takes
/// the actor and tenant from the connection (Absinthe's context per request and per
/// socket), an app attaches them here: typically a `Context` with the request's actor and
/// tenant, which every resolver reads as the request's.
pub trait RequestData: Send + Sync + 'static {
    /// `request`, with what `headers` say it runs as attached, and a scope naming that
    /// (the tenant and the actor, say): subscribers share a stream only within one scope,
    /// so one never hears what another may not see.
    fn apply(&self, headers: &HeaderMap, request: async_graphql::Request) -> (async_graphql::Request, String);
}

/// Every request runs as the schema's context says.
#[derive(Clone, Copy, Debug, Default)]
pub struct SchemaContext;

impl RequestData for SchemaContext {
    fn apply(&self, _headers: &HeaderMap, request: async_graphql::Request) -> (async_graphql::Request, String) {
        (request, String::new())
    }
}

#[derive(Clone)]
struct Served {
    schema: Arc<Schema>,
    request_data: Arc<dyn RequestData>,
}

/// Default GraphQL execution handler for Axum.
pub async fn graphql_handler(
    State(schema): State<Arc<Schema>>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    schema.execute(req.into_inner()).await.into()
}

/// [`graphql_handler`], running the request as its [`RequestData`] says.
async fn graphql_handler_with(State(served): State<Served>, headers: HeaderMap, req: GraphQLRequest) -> GraphQLResponse {
    let (request, _) = served.request_data.apply(&headers, req.into_inner());
    served.schema.execute(request).await.into()
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
    graphql_router_with(schema, hub, Arc::new(SchemaContext))
}

/// [`graphql_router_with_hub`], each request and subscription running as `request_data`
/// says from its headers: its actor and tenant.
pub fn graphql_router_with(schema: Schema, hub: Arc<SubscriptionHub>, request_data: Arc<dyn RequestData>) -> Router {
    let served = Served {
        schema: Arc::new(schema.clone()),
        request_data: Arc::clone(&request_data),
    };
    let subscriptions = move |protocol: GraphQLProtocol, headers: HeaderMap, upgrade: WebSocketUpgrade| {
        let (schema, hub, request_data) = (schema.clone(), Arc::clone(&hub), Arc::clone(&request_data));
        async move { crate::ws::upgrade(protocol, headers, upgrade, schema, hub, request_data) }
    };

    Router::new()
        .route("/graphql/ws", get(subscriptions))
        .route("/graphql", post(graphql_handler_with))
        .route("/graphql", get(graphiql_handler))
        .route("/graphiql", get(graphiql_handler))
        .with_state(served)
}
