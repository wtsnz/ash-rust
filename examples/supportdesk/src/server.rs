//! The desk's API: GraphQL, its subscriptions, and the JSON endpoints for what ash-graphql
//! doesn't serve yet. Every request runs as the tenant and actor its headers name.

use std::sync::Arc;

use ash_core::{
    Actor, BulkCreateOptions, BulkDestroyOptions, BulkUpdateOptions, Context, Error, FieldMap, Resource,
    TransactionSupport, Value,
};
use ash_graphql::AshGraphQL;
use ash_graphql::axum::{RequestData, SubscriptionHub};
use ash_pubsub::PubSub;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{Agent, AuditEvent, DESK_DEF, Ticket};

/// The context a request runs in: `base`, in the tenant `x-org` names, acting as the
/// agent `x-actor` and `x-role` describe.
pub fn request_context<D>(base: &Context<D>, headers: &HeaderMap) -> Context<D> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let mut ctx = base.clone();
    if let Some(org) = header("x-org") {
        ctx = ctx.with_tenant(org);
    }
    if let (Some(id), Some(role)) = (header("x-actor").and_then(|id| Uuid::parse_str(id).ok()), header("x-role")) {
        ctx = ctx.with_actor(Actor::new(id).with_role(role));
    }
    ctx
}

/// GraphQL requests and subscriptions run as their headers say.
struct Headers<D>(Context<D>);

impl<D: Send + Sync + 'static> RequestData for Headers<D> {
    fn apply(&self, headers: &HeaderMap, request: async_graphql::Request) -> (async_graphql::Request, String) {
        let ctx = request_context(&self.0, headers);
        let scope = format!(
            "{}|{}",
            ctx.tenant().unwrap_or_default(),
            ctx.actor.as_ref().map(|a| format!("{}:{}", a.id, a.role().unwrap_or_default())).unwrap_or_default()
        );
        (request.data(ctx), scope)
    }
}

struct App<D> {
    base: Context<D>,
    seeded_ms: u64,
}

/// The API over `base`, whose notifier publishes to `pubsub`. `seeded_ms` is how long
/// loading the fixture took, for `GET /health/seeded`.
pub fn router<D: TransactionSupport + 'static>(
    base: Context<D>,
    pubsub: PubSub,
    seeded_ms: u64,
) -> Result<Router, async_graphql::dynamic::SchemaError> {
    let schema = AshGraphQL::builder(&DESK_DEF)
        .with_pubsub(pubsub)
        .with_dataloader()
        .mutation_action::<Ticket, D>("route_ticket", "route")
        .finish_with_context(base.clone())?;
    let graphql = ash_graphql::axum::graphql_router_with(
        schema,
        Arc::new(SubscriptionHub::default()),
        Arc::new(Headers(base.clone())),
    );
    let rpc_base = base.clone();
    let rpc = ash_typescript::rpc::axum::rpc_router(rpc::<D>(), move |headers| request_context(&rpc_base, headers));
    let app = Arc::new(App { base, seeded_ms });
    let api = Router::new()
        .route("/api/route", post(route::<D>))
        .route("/api/bulk", post(bulk::<D>))
        .route("/api/edit", post(edit::<D>))
        .route("/health", get(|| async { "OK" }))
        .route("/health/seeded", get(seeded::<D>))
        .with_state(app);
    Ok(graphql.merge(rpc).merge(api))
}

async fn seeded<D>(State(app): State<Arc<App<D>>>) -> Json<serde_json::Value> {
    Json(json!({ "seeded_ms": app.seeded_ms }))
}

/// An error as JSON, with the status it means.
struct Failure(Error);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code) = match &self.0 {
            Error::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Error::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Error::StaleRecord { .. } => (StatusCode::CONFLICT, "stale_record"),
            Error::Validation { .. } | Error::Invalid(_) | Error::Missing { .. } | Error::Multiple(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "invalid")
            }
            Error::TenantRequired { .. } => (StatusCode::BAD_REQUEST, "tenant_required"),
            _ => (StatusCode::INTERNAL_SERVER_ERROR, "error"),
        };
        (status, Json(json!({ "error": code, "message": self.0.to_string() }))).into_response()
    }
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        Failure(err)
    }
}

#[derive(Deserialize)]
struct CommentInput {
    body: String,
    #[serde(default)]
    internal: bool,
}

fn comment_fields(comments: Vec<CommentInput>) -> Vec<FieldMap> {
    comments
        .into_iter()
        .map(|c| {
            let mut fields = FieldMap::new();
            fields.insert("body".into(), Value::String(c.body));
            fields.insert("internal".into(), Value::Bool(c.internal));
            fields
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteInput {
    subject: String,
    body: String,
    priority: i64,
    requester_email: String,
    #[serde(default)]
    comments: Vec<CommentInput>,
}

/// `POST /api/route`: the `route` generic action. Returns the routed ticket's id.
async fn route<D: TransactionSupport + 'static>(
    State(app): State<Arc<App<D>>>,
    headers: HeaderMap,
    Json(input): Json<RouteInput>,
) -> Result<Json<serde_json::Value>, Failure> {
    let ctx = request_context(&app.base, &headers);
    let id = route_ticket(
        &ctx,
        input.subject,
        input.body,
        input.priority,
        input.requester_email,
        comment_fields(input.comments),
    )
    .await?;
    Ok(Json(json!({ "id": id })))
}

/// Runs the `route` generic action, for the JSON endpoint.
pub async fn route_ticket<D: TransactionSupport + 'static>(
    ctx: &Context<D>,
    subject: String,
    body: String,
    priority: i64,
    requester_email: String,
    comments: Vec<FieldMap>,
) -> ash_core::Result<Uuid> {
    Ticket::route(ctx)
        .subject(subject)
        .body(body)
        .priority(priority)
        .requester_email(requester_email)
        .comments(comments)
        .call()
        .await
}

/// The actions the desk serves over AshTypescript's RPC, as the Elixir desk's
/// `typescript_rpc` block declares them.
pub fn rpc<D: TransactionSupport + 'static>() -> ash_typescript::rpc::Rpc<D> {
    use crate::{Comment, Tag};
    ash_typescript::rpc::Rpc::new()
        .action::<Ticket>("list_tickets", "read")
        .get_by::<Ticket>("get_ticket", "read", &["id"])
        .action::<Ticket>("open_ticket", "open")
        .action::<Ticket>("assign_ticket", "assign")
        .action::<Ticket>("start_ticket", "start")
        .action::<Ticket>("hold_ticket", "hold")
        .action::<Ticket>("resolve_ticket", "resolve")
        .action::<Ticket>("reopen_ticket", "reopen")
        .action::<Ticket>("close_ticket", "close")
        .action::<Ticket>("view_ticket", "view")
        .action::<Ticket>("edit_ticket", "edit")
        .action::<Ticket>("destroy_ticket", "destroy")
        .action::<Ticket>("route_ticket", "route")
        .action::<Comment>("list_comments", "read")
        .action::<Comment>("create_comment", "create")
        .action::<Agent>("list_agents", "read")
        .action::<Tag>("list_tags", "read")
        .action::<AuditEvent>("list_audit_events", "read")
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BulkInput {
    count: usize,
    assignee_id: Option<Uuid>,
}

/// `POST /api/bulk`: creates `count` tickets, assigns them all, then destroys them, each
/// with a bulk action. Bulk actions don't notify, as Ash's don't by default.
async fn bulk<D: TransactionSupport + 'static>(
    State(app): State<Arc<App<D>>>,
    headers: HeaderMap,
    Json(input): Json<BulkInput>,
) -> Result<Json<serde_json::Value>, Failure> {
    let ctx = request_context(&app.base, &headers);
    let inputs: Vec<FieldMap> = (0..input.count)
        .map(|i| {
            let mut fields = FieldMap::new();
            fields.insert("subject".into(), Value::String(format!("Bulk import {i}")));
            fields.insert("body".into(), Value::String("Imported from the old desk".into()));
            fields.insert("priority".into(), Value::Int(1 + (i % 4) as i64));
            fields.insert("requester_email".into(), Value::String(format!("import{i}@example.com")));
            fields.insert("comments".into(), Value::Array(Vec::new()));
            fields
        })
        .collect();
    let created = Ticket::bulk_create_with_opts(&ctx, "open", inputs, BulkCreateOptions::new().notify(false)).await?;
    let mut assign = FieldMap::new();
    assign.insert("assignee_id".into(), input.assignee_id.map(Value::Uuid).unwrap_or(Value::Null));
    let updates = created.records.iter().cloned().map(|ticket| (ticket, assign.clone()));
    let updated = Ticket::bulk_update_with_opts(&ctx, "assign", updates, BulkUpdateOptions::new().notify(false)).await?;
    let ids: Vec<Uuid> = created.records.iter().map(Resource::id).collect();
    let destroyed = Ticket::bulk_destroy_with_opts(
        &ctx,
        "destroy",
        &ids,
        BulkDestroyOptions::new().notify(false).return_records(false),
    )
    .await?;
    Ok(Json(json!({ "created": created.count, "updated": updated.count, "destroyed": destroyed.count })))
}

#[derive(Deserialize)]
struct EditInput {
    id: Uuid,
    version: i64,
    subject: Option<String>,
    priority: Option<i64>,
}

/// `POST /api/edit`: edits a ticket as of the version the client read, failing as stale
/// if it has changed since. Returns the new version.
async fn edit<D: TransactionSupport + 'static>(
    State(app): State<Arc<App<D>>>,
    headers: HeaderMap,
    Json(input): Json<EditInput>,
) -> Result<Json<serde_json::Value>, Failure> {
    let ctx = request_context(&app.base, &headers);
    let mut ticket = Ticket::get(&ctx, input.id).await?;
    ticket.version = input.version;
    let mut edit = ticket.edit_on(&ctx);
    if let Some(subject) = input.subject {
        edit = edit.subject(subject);
    }
    if let Some(priority) = input.priority {
        edit = edit.priority(priority);
    }
    let edited = edit.await?;
    Ok(Json(json!({ "version": edited.version })))
}
