//! The HTTP API: password sign-in that issues a JWT, and GraphQL over both databases.
//!
//! - `POST /auth/sign-in` with `{"email", "password"}` returns `{"token", "line", "role"}`.
//! - `POST /graphql` runs a query as the token's crew member, in their line, or as an
//!   anonymous reader without a token.
//! - `GET /graphql` serves GraphiQL, and `GET /graphql/ws` serves subscriptions.

use std::sync::Arc;

use ash_authentication::JwtService;
use ash_core::{Actor, Context, Resource, StoreRegistry};
use async_graphql::dynamic::Schema;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::Lagrange;

/// Every resource the API serves, from all four domains.
pub fn api_resources() -> Vec<&'static ash_core::ResourceDef> {
    [
        &crate::WORLD_DEF,
        &crate::FLEET_DEF,
        &crate::CARGO_DEF,
        &crate::TELEMETRY_DEF,
    ]
    .into_iter()
    .flat_map(|domain| domain.resources.iter().copied())
    .collect()
}

/// The GraphQL schema on the registry, so telemetry and fleet resources are both
/// reachable. Subscriptions use the platform's pubsub.
pub fn schema<D>(app: &Lagrange<D>) -> Result<Schema, async_graphql::dynamic::SchemaError>
where
    D: ash_core::DataLayer + Clone + Send + Sync + 'static,
{
    ash_graphql::AshGraphQL::from_resources(&api_resources())
        .with_pubsub(app.pubsub().clone())
        .with_dataloader()
        .finish_with_context(app.registry_context())
}

struct ApiState<D> {
    app: Arc<Lagrange<D>>,
    schema: Schema,
    jwt: Arc<JwtService>,
}

#[derive(Deserialize)]
struct SignIn {
    email: String,
    password: String,
}

#[derive(Serialize)]
struct Session {
    token: String,
    line: String,
    role: String,
}

/// The routes, sharing one platform and token signer.
pub fn router<D>(app: Arc<Lagrange<D>>, jwt: Arc<JwtService>) -> Result<Router, async_graphql::dynamic::SchemaError>
where
    D: ash_core::DataLayer + Clone + Send + Sync + 'static,
{
    let schema = schema(&app)?;
    let subscriptions = GraphQLSubscription::new(schema.clone());
    let state = Arc::new(ApiState { app, schema, jwt });
    Ok(Router::new()
        .route("/auth/sign-in", post(sign_in::<D>))
        .route("/graphql", post(graphql::<D>).get(graphiql))
        .route_service("/graphql/ws", subscriptions)
        .route("/health", get(|| async { "OK" }))
        .with_state(state))
}

async fn sign_in<D>(
    State(state): State<Arc<ApiState<D>>>,
    Json(request): Json<SignIn>,
) -> Result<Json<Session>, StatusCode>
where
    D: ash_core::DataLayer + Clone + Send + Sync + 'static,
{
    let crew = state
        .app
        .authenticate(&request.email, &request.password)
        .await
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let role = match ash_core::AshType::to_value(&crew.role) {
        ash_core::Value::String(role) => role,
        _ => return Err(StatusCode::INTERNAL_SERVER_ERROR),
    };
    let token = state
        .jwt
        .sign_access_token(crew.id(), Some(role.clone()), Some(crew.line.clone()))
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(Session {
        token,
        line: crew.line,
        role,
    }))
}

/// The request's context: the token's crew member in their line, or nobody.
fn request_context<D>(state: &ApiState<D>, headers: &HeaderMap) -> Result<Context<StoreRegistry>, StatusCode>
where
    D: ash_core::DataLayer + Clone + Send + Sync + 'static,
{
    let ctx = state.app.registry_context();
    let Some(header) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Ok(ctx);
    };
    let token = header
        .to_str()
        .ok()
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let claims = state
        .jwt
        .verify_token_with_purpose(token, "access")
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    let mut actor = Actor::new(claims.sub);
    if let Some(role) = claims.role {
        actor = actor.with_role(role);
    }
    let mut ctx = ctx.with_actor(actor);
    if let Some(line) = claims.tenant {
        ctx = ctx.with_tenant(line);
    }
    Ok(ctx)
}

async fn graphql<D>(
    State(state): State<Arc<ApiState<D>>>,
    headers: HeaderMap,
    request: GraphQLRequest,
) -> Result<GraphQLResponse, StatusCode>
where
    D: ash_core::DataLayer + Clone + Send + Sync + 'static,
{
    let ctx = request_context(&state, &headers)?;
    Ok(state.schema.execute(request.into_inner().data(ctx)).await.into())
}

async fn graphiql() -> impl IntoResponse {
    Html(
        async_graphql::http::GraphiQLSource::build()
            .endpoint("/graphql")
            .subscription_endpoint("/graphql/ws")
            .finish(),
    )
}

/// The TypeScript SDK for every resource the API serves, with Zod schemas when `zod`.
pub fn typescript_sdk(zod: bool) -> Result<String, ash_typescript::CodegenError> {
    ash_typescript::TypeScriptGenerator::new()
        .config(
            ash_typescript::TypeScriptConfig::new()
                .with_zod(zod)
                .with_client(true)
                .with_client_name("LagrangeClient")
                .with_graphql_endpoint("/graphql"),
        )
        .add_domain(&crate::WORLD_DEF)
        .add_domain(&crate::FLEET_DEF)
        .add_domain(&crate::CARGO_DEF)
        .add_domain(&crate::TELEMETRY_DEF)
        .generate_consolidated()
}
