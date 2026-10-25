//! Axum web framework integration helpers for `ash-graphql`.

use async_graphql::dynamic::Schema;
use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLProtocol, GraphQLRequest, GraphQLResponse};
use axum::{
    Router,
    extract::{State, WebSocketUpgrade},
    http::HeaderMap,
    response::{Html, IntoResponse},
    routing::{get, post},
};
use std::sync::Arc;

pub use crate::ws::{Hub as SubscriptionHub, SHARED_BUFFER};

/// A server-bound request and its subscription execution scope. HTTP operations
/// retain the member context. An explicitly supplied group replaces it only for
/// subscriptions, so shared results always resolve as the group's actor.
pub struct BoundRequest {
    request: async_graphql::Request,
    group: Option<SubscriptionGroup>,
}

#[derive(Clone)]
struct SubscriptionGroup {
    scope: ash_core::SubscriptionScope,
    bind: Arc<dyn Fn(async_graphql::Request) -> async_graphql::Request + Send + Sync>,
}
impl BoundRequest {
    pub fn new<D: Send + Sync + 'static>(
        request: async_graphql::Request,
        ctx: ash_core::Context<D>,
    ) -> Self {
        let scope = ctx.subscription_scope();
        let group_ctx = ctx.clone();
        Self {
            request: request.data(ctx),
            group: Some(SubscriptionGroup {
                scope,
                bind: Arc::new(move |request| request.data(group_ctx.clone())),
            }),
        }
    }
    /// Declare the generic actor and tenant used by every member of this group.
    /// Keep and clone this bound context; binding a new authenticated context creates
    /// another group, even when its actor has the same ID. Both contexts must use
    /// the same backend type, so the group always replaces the resolver's identity.
    ///
    /// ```compile_fail
    /// use ash_graphql::axum::BoundRequest;
    /// let member = ash_core::Runtime::new(()).bootstrap();
    /// BoundRequest::for_subscription_group(async_graphql::Request::new("subscription { event }"), member, "staff");
    /// ```
    pub fn for_subscription_group<D: Send + Sync + 'static>(
        request: async_graphql::Request,
        member: ash_core::Context<D>,
        ctx: ash_core::Context<D>,
    ) -> Self {
        let mut bound = Self::new(request, member);
        bound.group = Some(SubscriptionGroup {
            scope: ctx.subscription_scope(),
            bind: Arc::new(move |request| request.data(ctx.clone())),
        });
        bound
    }
    pub fn unbound(request: async_graphql::Request) -> Self {
        Self {
            request,
            group: None,
        }
    }
    pub fn into_request(self) -> async_graphql::Request {
        self.request
    }
    pub(crate) fn subscription(
        self,
    ) -> (async_graphql::Request, Option<ash_core::SubscriptionScope>) {
        match self.group {
            Some(group) => ((group.bind)(self.request), Some(group.scope)),
            None => (self.request, None),
        }
    }
}

/// Bind actor and tenant in trusted server code. App-defined strings cannot authorize
/// result sharing. Both WebSocket protocols use this binder.
pub trait RequestData: Send + Sync + 'static {
    fn apply(&self, _headers: &HeaderMap, request: async_graphql::Request) -> BoundRequest {
        BoundRequest::unbound(request)
    }

    /// Authenticate against durable storage before binding each network operation.
    fn apply_async<'a>(
        &'a self,
        headers: &'a HeaderMap,
        request: async_graphql::Request,
    ) -> std::pin::Pin<Box<dyn Future<Output = BoundRequest> + Send + 'a>> {
        Box::pin(async move { self.apply(headers, request) })
    }

    /// Async counterpart of connection_init for durable authentication.
    fn connection_init_async<'a>(
        &'a self,
        headers: &'a HeaderMap,
        params: Option<&'a serde_json::Value>,
    ) -> std::pin::Pin<Box<dyn Future<Output = ash_core::Result<HeaderMap>> + Send + 'a>> {
        Box::pin(async move { self.connection_init(headers, params) })
    }

    /// Validate connection-init credentials and return the headers used for this
    /// connection's binding. Applications define credential precedence here. The
    /// default rejects nonempty parameters instead of silently ignoring credentials.
    fn connection_init(
        &self,
        headers: &HeaderMap,
        params: Option<&serde_json::Value>,
    ) -> ash_core::Result<HeaderMap> {
        if params.is_some_and(|params| {
            !params.is_null() && params.as_object().is_none_or(|map| !map.is_empty())
        }) {
            return Err(ash_core::Error::Invalid(
                "connection-init credentials are not supported by this binder".into(),
            ));
        }
        Ok(headers.clone())
    }
}
impl<D: Send + Sync + 'static> RequestData for ash_core::Context<D> {
    fn apply(&self, _headers: &HeaderMap, request: async_graphql::Request) -> BoundRequest {
        BoundRequest::new(request, self.clone())
    }
}

/// Network execution without a binding fails instead of using the schema default.
#[derive(Clone, Copy, Debug, Default)]
pub struct SchemaContext;
impl RequestData for SchemaContext {
    fn apply(&self, _headers: &HeaderMap, request: async_graphql::Request) -> BoundRequest {
        BoundRequest::unbound(request)
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
    schema
        .execute(req.into_inner().data(crate::request::RequireBoundContext))
        .await
        .into()
}

/// [`graphql_handler`], running the request as its [`RequestData`] says.
async fn graphql_handler_with(
    State(served): State<Served>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let request = served
        .request_data
        .apply_async(&headers, req.into_inner())
        .await
        .into_request();
    served
        .schema
        .execute(request.data(crate::request::RequireBoundContext))
        .await
        .into()
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
pub fn graphql_router<D: Send + Sync + 'static>(
    schema: Schema,
    context: ash_core::Context<D>,
) -> Router {
    graphql_router_with_hub(schema, Arc::new(SubscriptionHub::default()), context)
}

/// [`graphql_router`], with the [`SubscriptionHub`] its shared subscriptions run in, to
/// see how many are running.
pub fn graphql_router_with_hub<D: Send + Sync + 'static>(
    schema: Schema,
    hub: Arc<SubscriptionHub>,
    context: ash_core::Context<D>,
) -> Router {
    graphql_router_with(schema, hub, Arc::new(context))
}

/// [`graphql_router_with_hub`], each request and subscription running as `request_data`
/// says from its headers: its actor and tenant.
pub fn graphql_router_with(
    schema: Schema,
    hub: Arc<SubscriptionHub>,
    request_data: Arc<dyn RequestData>,
) -> Router {
    let served = Served {
        schema: Arc::new(schema.clone()),
        request_data: Arc::clone(&request_data),
    };
    Router::new()
        .route("/graphql", post(graphql_handler_with))
        .route("/graphql", get(graphiql_handler))
        .route("/graphiql", get(graphiql_handler))
        .with_state(served)
        .merge(graphql_subscription_router_with(schema, hub, request_data))
}

/// Only the bound `/graphql/ws` endpoint, for applications with custom HTTP handlers.
/// Merge this Router after supplying the custom handlers' state.
pub fn graphql_subscription_router_with(
    schema: Schema,
    hub: Arc<SubscriptionHub>,
    request_data: Arc<dyn RequestData>,
) -> Router {
    let subscriptions =
        move |protocol: GraphQLProtocol, headers: HeaderMap, upgrade: WebSocketUpgrade| {
            let (schema, hub, request_data) =
                (schema.clone(), Arc::clone(&hub), Arc::clone(&request_data));
            async move { crate::ws::upgrade(protocol, headers, upgrade, schema, hub, request_data) }
        };

    Router::new().route("/graphql/ws", get(subscriptions))
}
