use std::sync::Arc;

use ash_core::{Actor, Context, DataLayer, Resource};
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AuthError;
use crate::strategy::AuthStrategy;
use crate::token::JwtService;

/// Axum extractor providing the authenticated `Actor` from an `Authorization: Bearer <token>` header.
#[derive(Clone, Debug)]
pub struct AuthUser(pub Actor);

impl<S> FromRequestParts<S> for AuthUser
where
    S: Send + Sync,
    Arc<JwtService>: axum::extract::FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        use axum::extract::FromRef;
        let jwt = Arc::<JwtService>::from_ref(state);

        let auth_header = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or(AuthRejection::MissingHeader)?;

        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or(AuthRejection::InvalidHeader)?;

        let claims = jwt
            .verify_token(token)
            .map_err(|e| AuthRejection::InvalidToken(e.to_string()))?;

        let mut actor = Actor::new(claims.sub);
        if let Some(role) = claims.role {
            actor = actor.with_role(role);
        }

        Ok(AuthUser(actor))
    }
}

/// HTTP rejections returned when token extraction or authentication fails.
#[derive(Debug)]
pub enum AuthRejection {
    MissingHeader,
    InvalidHeader,
    InvalidToken(String),
    AuthError(AuthError),
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        let (status, message): (StatusCode, String) = match self {
            Self::MissingHeader => (
                StatusCode::UNAUTHORIZED,
                "missing authorization header".to_string(),
            ),
            Self::InvalidHeader => (
                StatusCode::UNAUTHORIZED,
                "invalid authorization header format (expected Bearer <token>)".to_string(),
            ),
            Self::InvalidToken(msg) => (StatusCode::UNAUTHORIZED, msg),
            Self::AuthError(err) => (StatusCode::UNAUTHORIZED, err.to_string()),
        };

        let body = Json(serde_json::json!({ "error": message }));
        (status, body).into_response()
    }
}

/// DTO for password authentication requests.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PasswordAuthRequest {
    pub identity: String,
    pub password: String,
}

/// DTO for successful authentication responses containing a bearer token and user ID.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthTokenResponse {
    pub token: String,
    pub user_id: Uuid,
    pub role: Option<String>,
}

/// State container for the authentication router.
pub struct AuthRouterState<R: Resource, D: DataLayer> {
    pub strategy: AuthStrategy<R>,
    pub jwt_service: Arc<JwtService>,
    pub context: Context<D>,
}

/// Build a standard `/auth` router with `POST /sign-in`, `POST /revoke`, and `GET /me`.
pub fn auth_router<R, D, S>(
    strategy: AuthStrategy<R>,
    jwt_service: Arc<JwtService>,
    context: Context<D>,
) -> Router<S>
where
    R: Resource + Send + Sync + 'static,
    D: DataLayer + Send + Sync + 'static,
    S: Clone + Send + Sync + 'static,
{
    let state = Arc::new(AuthRouterState {
        strategy: strategy.with_jwt_service((*jwt_service).clone()),
        jwt_service,
        context,
    });

    Router::new()
        .route("/sign-in", post(sign_in_handler::<R, D>))
        .route("/me", get(me_handler::<R, D>))
        .with_state(state)
}

async fn sign_in_handler<R: Resource, D: DataLayer>(
    State(state): State<Arc<AuthRouterState<R, D>>>,
    Json(payload): Json<PasswordAuthRequest>,
) -> Result<Json<AuthTokenResponse>, AuthRejection> {
    let (record, token) = state
        .strategy
        .sign_in_with_password_and_token(&state.context, payload.identity, &payload.password)
        .await
        .map_err(AuthRejection::AuthError)?;

    let fields = record.to_fields();
    let role = match fields.get("role") {
        Some(ash_core::Value::String(r)) => Some(r.clone()),
        _ => None,
    };

    Ok(Json(AuthTokenResponse {
        token,
        user_id: record.id(),
        role,
    }))
}

async fn me_handler<R: Resource, D: DataLayer>(
    State(state): State<Arc<AuthRouterState<R, D>>>,
    headers: axum::http::HeaderMap,
) -> Result<Json<serde_json::Value>, AuthRejection> {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AuthRejection::MissingHeader)?;

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or(AuthRejection::InvalidHeader)?;

    let claims = state
        .jwt_service
        .verify_token(token)
        .map_err(|e| AuthRejection::InvalidToken(e.to_string()))?;

    Ok(Json(serde_json::json!({
        "user_id": claims.sub,
        "role": claims.role,
        "tenant": claims.tenant,
    })))
}
