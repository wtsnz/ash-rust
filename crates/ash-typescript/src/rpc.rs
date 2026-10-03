//! AshTypescript's RPC: actions run by name over one endpoint, wire-compatible with
//! `AshTypescript.Rpc.run_action`, so a client AshTypescript generates works against an
//! ash-rust server as against an Elixir one.
//!
//! A request is JSON: `action` (the RPC action's name), and as the action needs them
//! `fields`, `input`, `identity`, `getBy`, `filter`, `sort`, `page` and `tenant`. Field
//! names are camelCase on the wire, snake_case in the resource. The answer is always
//! `{"success": true, "data": …}` or `{"success": false, "errors": [{"type", "message",
//! "shortMessage", "vars", "fields", "path"}]}`.
//!
//! - `fields` selects what comes back: attribute, aggregate and calculation names, and
//!   relationships as objects, `{"assignee": ["name"]}`, or with their own read,
//!   `{"comments": {"fields": ["body"], "filter": …, "sort": "-insertedAt", "limit": 3}}`.
//! - A read answers a list; with `page` it answers a page: `{"limit", "offset", "count"}`
//!   an offset page, any other a keyset page (`after`, `before`, `nextPage`,
//!   `previousPage`). A `get_by` action answers one record, or `not_found`.
//! - A create, update or destroy answers the selected fields of the record (`{}` with no
//!   `fields`); an update or destroy finds it by `identity`, its primary key, through the
//!   read action as the actor: one it can't read is `not_found`.
//! - A generic action answers what it returns.
//!
//! Reads run through ash-core as every other read does: the action's preparations, the
//! actor's policies and the tenant, relationship loads with their own policies, and
//! fields the actor may not read redacted.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use ash_core::input::{action_input, field_type, filter_input, sort_input};
use ash_core::{
    ActionDef, ActionKind, CompiledQuery, Context, DataLayer, Error, FieldMap, Filter, KeysetCursor, RelKind,
    RelatedQuery, Resource, ResourceDef, Sort, TransactionSupport, Value,
};
use serde_json::{Map, Value as Json, json};

use crate::types::to_camel_case;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What a generic action does, supplied with its RPC action: the context it runs in, and
/// its input, cast to its arguments' types.
pub type GenericRunner<D> = Arc<dyn Fn(Context<D>, FieldMap) -> BoxFuture<'static, ash_core::Result<Value>> + Send + Sync>;

/// An action clients run by name.
struct RpcAction<D> {
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    /// The attributes a `get_by` action finds its one record by.
    get_by: Vec<&'static str>,
    runner: Option<GenericRunner<D>>,
}

/// The actions an app exposes over RPC, as a domain's `typescript_rpc` block declares
/// them in AshTypescript.
pub struct Rpc<D> {
    actions: HashMap<String, RpcAction<D>>,
}

impl<D> Default for Rpc<D> {
    fn default() -> Self {
        Self { actions: HashMap::new() }
    }
}

fn action_def(resource: &'static ResourceDef, action: &str) -> &'static ActionDef {
    resource
        .actions
        .iter()
        .find(|a| a.name == action)
        .unwrap_or_else(|| panic!("{} has no action `{action}`", resource.name))
}

impl<D: TransactionSupport + 'static> Rpc<D> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `rpc_action :name, :action`: a read, create, update or destroy action of `R`.
    pub fn action<R: Resource>(mut self, name: &str, action: &str) -> Self {
        let action = action_def(&R::DEF, action);
        assert!(action.kind != ActionKind::Generic, "`{name}` is a generic action: expose it with `generic`");
        self.actions.insert(name.into(), RpcAction { resource: &R::DEF, action, get_by: Vec::new(), runner: None });
        self
    }

    /// `rpc_action :name, :read, get_by: fields`: one record of `R`, found by `fields`.
    pub fn get_by<R: Resource>(mut self, name: &str, action: &str, fields: &[&'static str]) -> Self {
        let action = action_def(&R::DEF, action);
        self.actions.insert(name.into(), RpcAction { resource: &R::DEF, action, get_by: fields.to_vec(), runner: None });
        self
    }

    /// `rpc_action :name, :generic_action`: a generic action of `R`, which `run` performs.
    /// A generic action's work is given here, where the data layer is known, since one
    /// written in the resource's DSL can't open a transaction.
    pub fn generic<R: Resource, F, Fut>(mut self, name: &str, action: &str, run: F) -> Self
    where
        F: Fn(Context<D>, FieldMap) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ash_core::Result<Value>> + Send + 'static,
    {
        let action = action_def(&R::DEF, action);
        let runner: GenericRunner<D> = Arc::new(move |ctx, input| Box::pin(run(ctx, input)));
        self.actions.insert(name.into(), RpcAction { resource: &R::DEF, action, get_by: Vec::new(), runner: Some(runner) });
        self
    }

    /// Runs `request` in `ctx`, its `tenant` overriding the context's, and answers as
    /// AshTypescript does.
    pub async fn run(&self, ctx: &Context<D>, request: &Json) -> Json {
        match self.answer(ctx, request).await {
            Ok(data) => json!({ "success": true, "data": data }),
            Err(failure) => json!({ "success": false, "errors": [failure.to_json()] }),
        }
    }

    async fn answer(&self, ctx: &Context<D>, request: &Json) -> Result<Json, Failure> {
        let name = request["action"].as_str().unwrap_or_default();
        let rpc = self.actions.get(name).ok_or_else(|| Failure::action_not_found(name))?;
        let mut ctx = ctx.clone();
        if let Some(tenant) = request["tenant"].as_str() {
            ctx = ctx.with_tenant(tenant);
        }
        let selection = match &request["fields"] {
            Json::Null => Selection::default(),
            fields => Selection::parse(rpc.resource, ctx.actor.as_ref(), fields)?,
        };
        let (resource, action) = (rpc.resource, rpc.action);
        match action.kind {
            ActionKind::Read => self.read(&ctx, rpc, &selection, request).await,
            ActionKind::Create => {
                let input = action_input(resource, action, &snake_keys(&request["input"]))?;
                let stored = ash_core::create_dynamic(&ctx, resource, action, input).await?;
                Ok(written(&ctx, resource, stored, &selection).await?)
            }
            ActionKind::Update => {
                let id = identity(resource, &request["identity"])?;
                let input = action_input(resource, action, &snake_keys(&request["input"]))?;
                let stored = ash_core::update_dynamic(&ctx, resource, action, id, input).await?;
                Ok(written(&ctx, resource, stored, &selection).await?)
            }
            ActionKind::Destroy => {
                let id = identity(resource, &request["identity"])?;
                let stored = ash_core::destroy_dynamic_by_id(&ctx, resource, action, id, None).await?;
                Ok(written(&ctx, resource, stored, &selection).await?)
            }
            ActionKind::Generic => {
                let runner = rpc.runner.as_ref().ok_or_else(|| Failure::action_not_found(name))?;
                let input = action_input(resource, action, &snake_keys(&request["input"]))?;
                Ok(to_json(&runner(ctx, input).await?))
            }
        }
    }

    async fn read(&self, ctx: &Context<D>, rpc: &RpcAction<D>, selection: &Selection, request: &Json) -> Result<Json, Failure> {
        let resource = rpc.resource;
        let mut filters = Vec::new();
        if !request["filter"].is_null() {
            let filter = filter_input(resource, &snake_keys(&request["filter"]))?;
            filters.push(ash_core::guard_input_filter(resource, ctx.actor.as_ref(), filter)?);
        }
        let get = !rpc.get_by.is_empty();
        if get {
            for field in &rpc.get_by {
                let given = &request["getBy"][to_camel_case(field)];
                let ty = field_type(resource, field).expect("get_by names attributes");
                filters.push(Filter::eq(*field, ash_core::input::value_input(ty, given)?));
            }
        }
        let sort = match request["sort"].as_str() {
            Some(text) => ash_core::guard_input_sort(resource, ctx.actor.as_ref(), sort_input(resource, &snake_sort(text))?)?,
            None => Vec::new(),
        };
        let arguments = action_input(resource, rpc.action, &snake_keys(&request["input"]))?;
        let base = CompiledQuery {
            filter: Some(Filter::and(filters)),
            sort,
            tenant: ctx.tenant.clone(),
            ..CompiledQuery::default()
        };
        let scoped = ash_core::scope_read(resource, rpc.action, ctx.actor.as_ref(), &arguments, base)?;
        if get {
            let rows = read_rows(ctx, resource, selection, CompiledQuery { limit: Some(2), ..scoped }).await?;
            return match rows.len() {
                0 => Err(Error::NotFound.into()),
                1 => Ok(render(ctx, resource, rows, selection).await?.remove(0)),
                n => Err(Error::TooMany(n).into()),
            };
        }
        let page = &request["page"];
        let limit = page["limit"].as_u64().map(|n| n as usize);
        let Some(limit) = limit.filter(|_| page.is_object()) else {
            let rows = read_rows(ctx, resource, selection, scoped).await?;
            return Ok(Json::Array(render(ctx, resource, rows, selection).await?));
        };
        let count = if page["count"].as_bool() == Some(true) {
            let counted = CompiledQuery { sort: Vec::new(), limit: None, offset: None, ..scoped.clone() };
            Some(ctx.data.count(resource, &counted).await?)
        } else {
            None
        };
        if let Some(offset) = page["offset"].as_u64() {
            let offset = offset as usize;
            let mut rows = read_rows(ctx, resource, selection, CompiledQuery { limit: Some(limit + 1), offset: Some(offset), ..scoped }).await?;
            let has_more = rows.len() > limit;
            rows.truncate(limit);
            let results = render(ctx, resource, rows, selection).await?;
            return Ok(json!({
                "results": results, "hasMore": has_more, "limit": limit, "offset": offset, "count": count, "type": "offset",
            }));
        }
        // A keyset page: stable, and from a cursor when given one.
        let sort = ash_core::keyset_sort(resource, scoped.sort.clone());
        let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");
        let (after, before) = (page["after"].as_str(), page["before"].as_str());
        let backward = before.is_some() && after.is_none();
        let mut keyset_filter = scoped.filter.clone();
        if let Some(cursor) = after.or(before).and_then(KeysetCursor::decode) {
            let values = ash_core::keyset_values(resource, &cursor, &sort);
            if let Some(keyset) = ash_core::build_keyset_filter(resource, &sort, &values, !backward) {
                keyset_filter = Some(Filter::and(keyset_filter.into_iter().chain([keyset])));
            }
        }
        let read_sort: Vec<Sort> = sort
            .iter()
            .map(|s| Sort { descending: s.descending != backward, ..s.clone() })
            .collect();
        let query = CompiledQuery { filter: keyset_filter, sort: read_sort, limit: Some(limit + 1), offset: None, ..scoped };
        let mut rows = read_rows(ctx, resource, &selection.keeping(sort.iter().map(|s| s.field.as_str())), query).await?;
        let has_more = rows.len() > limit;
        rows.truncate(limit);
        if backward {
            rows.reverse();
        }
        let cursor = |row: Option<&FieldMap>| -> Json {
            row.map_or(Json::Null, |row| {
                let id = row.get(pk).and_then(Value::as_uuid).unwrap_or_default();
                let values = sort.iter().map(|s| (s.field.clone(), row.get(&s.field).cloned().unwrap_or(Value::Null))).collect();
                Json::String(KeysetCursor { id, values }.encode())
            })
        };
        let (previous, next) = (cursor(rows.first()), cursor(rows.last()));
        let results = render(ctx, resource, rows, selection).await?;
        Ok(json!({
            "results": results, "hasMore": has_more, "limit": limit, "after": after, "before": before,
            "nextPage": next, "previousPage": previous, "count": count, "type": "keyset",
        }))
    }
}

/// An update's or destroy's record: `identity`, its primary key.
fn identity(resource: &ResourceDef, given: &Json) -> Result<uuid::Uuid, Failure> {
    let text = match given {
        Json::String(text) => text.as_str(),
        Json::Object(fields) => {
            let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");
            fields.get(&to_camel_case(pk)).and_then(Json::as_str).unwrap_or_default()
        }
        _ => "",
    };
    uuid::Uuid::parse_str(text).map_err(|_| Error::Invalid(format!("invalid identity {given}")).into())
}

/// What `fields` selects of a resource.
#[derive(Clone, Default)]
struct Selection {
    /// Attributes, aggregates and calculations, by name, in the order given.
    fields: Vec<String>,
    relationships: Vec<(String, Nested)>,
}

/// A relationship's selection, and how its rows are read.
#[derive(Clone)]
struct Nested {
    selection: Selection,
    query: RelatedQuery,
}

impl Selection {
    /// The selection `fields` makes of `resource`, its relationships' filters and sorts as
    /// `actor` may run them.
    fn parse(resource: &'static ResourceDef, actor: Option<&ash_core::Actor>, fields: &Json) -> Result<Self, Failure> {
        let items = fields.as_array().ok_or_else(|| Error::Invalid("fields must be a list".into()))?;
        let mut selection = Selection::default();
        for item in items {
            match item {
                Json::String(name) => {
                    let field = snake(name);
                    if resource.relationship(&field).is_some() {
                        return Err(Failure::requires_field_selection(name));
                    }
                    if field_type(resource, &field).is_none() {
                        return Err(Failure::unknown_field(name, resource));
                    }
                    selection.fields.push(field);
                }
                Json::Object(rels) => {
                    for (name, nested) in rels {
                        let rel_name = snake(name);
                        let rel = resource.relationship(&rel_name).ok_or_else(|| Failure::unknown_field(name, resource))?;
                        let dest = (rel.destination)();
                        let (fields, options) = match nested {
                            Json::Array(_) => (nested.clone(), Json::Null),
                            Json::Object(options) => (options.get("fields").cloned().unwrap_or(Json::Array(Vec::new())), nested.clone()),
                            _ => return Err(Failure::requires_field_selection(name)),
                        };
                        let inner = Selection::parse(dest, actor, &fields)?;
                        let filter = match &options["filter"] {
                            Json::Null => None,
                            filter => Some(ash_core::guard_input_filter(dest, actor, filter_input(dest, &snake_keys(filter))?)?),
                        };
                        let sort = match options["sort"].as_str() {
                            Some(text) => ash_core::guard_input_sort(dest, actor, sort_input(dest, &snake_sort(text))?)?,
                            None => Vec::new(),
                        };
                        let query = RelatedQuery {
                            filter,
                            sort,
                            limit: options["limit"].as_u64().map(|n| n as usize),
                            offset: options["offset"].as_u64().map(|n| n as usize),
                            select: Some(inner.attributes(dest)),
                            aggregates: inner.of(|name| dest.aggregate(name).is_some()),
                            calculations: inner.of(|name| dest.calculation(name).is_some()),
                        };
                        selection.relationships.push((rel_name, Nested { selection: inner, query }));
                    }
                }
                other => return Err(Error::Invalid(format!("invalid field selection {other}")).into()),
            }
        }
        Ok(selection)
    }

    fn of(&self, matches: impl Fn(&str) -> bool) -> Vec<String> {
        self.fields.iter().filter(|name| matches(name)).cloned().collect()
    }

    /// The attributes a read must select: those chosen, the primary key, those the field
    /// policies check (redacting the record reads them), and each chosen relationship's
    /// key on this side.
    fn attributes(&self, resource: &ResourceDef) -> Vec<String> {
        let mut attrs: Vec<String> = self.of(|name| resource.attribute(name).is_some());
        let mut keep = |name: &str| {
            if resource.attribute(name).is_some() && !attrs.iter().any(|a| a == name) {
                attrs.push(name.to_string());
            }
        };
        if let Some(pk) = resource.primary_key() {
            keep(pk.name);
        }
        for field in ash_core::field_policy_fields(resource) {
            keep(field);
        }
        for (rel_name, _) in &self.relationships {
            if let Some(rel) = resource.relationship(rel_name)
                && matches!(rel.kind, RelKind::BelongsTo)
            {
                for column in rel.source_columns() {
                    keep(column);
                }
            }
        }
        attrs
    }

    /// This selection, reading `fields` too: what a keyset needs of each record.
    fn keeping<'a>(&self, fields: impl Iterator<Item = &'a str>) -> Selection {
        let mut selection = self.clone();
        for field in fields {
            if !selection.fields.iter().any(|f| f == field) {
                selection.fields.push(field.to_string());
            }
        }
        selection
    }
}

/// The rows `query` reads of `resource`, selecting what `selection` needs, as the actor
/// may see them.
async fn read_rows<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    selection: &Selection,
    query: CompiledQuery,
) -> ash_core::Result<Vec<FieldMap>> {
    let query = CompiledQuery {
        select: Some(selection.attributes(resource)),
        aggregates: selection.of(|name| resource.aggregate(name).is_some()),
        calculations: selection.of(|name| resource.calculation(name).is_some()),
        ..query
    };
    let mut rows = ctx.data.run_query(resource, &query).await?;
    for row in &mut rows {
        ash_core::redact_fields(resource, ctx.actor.as_ref(), row)?;
    }
    Ok(rows)
}

/// A written record, as the client selected it: its aggregates and calculations read as
/// the actor sees them, and its relationships loaded.
async fn written<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    mut stored: FieldMap,
    selection: &Selection,
) -> ash_core::Result<Json> {
    let loads = selection.of(|name| resource.attribute(name).is_none());
    if !loads.is_empty()
        && let Some(pk) = resource.primary_key()
        && let Some(id) = stored.get(pk.name).cloned()
    {
        let query = CompiledQuery {
            filter: Some(Filter::eq(pk.name, id)),
            tenant: ctx.tenant.clone(),
            actor: ctx.actor.clone(),
            ..CompiledQuery::default()
        };
        if let Some(row) = read_rows(ctx, resource, selection, query).await?.into_iter().next() {
            for name in loads {
                if let Some(value) = row.get(&name) {
                    stored.insert(name, value.clone());
                }
            }
        }
    }
    Ok(render(ctx, resource, vec![stored], selection).await?.remove(0))
}

/// `rows` as the client selected them, their relationships loaded in turn.
fn render<'a, D: DataLayer>(
    ctx: &'a Context<D>,
    resource: &'static ResourceDef,
    rows: Vec<FieldMap>,
    selection: &'a Selection,
) -> BoxFuture<'a, ash_core::Result<Vec<Json>>> {
    Box::pin(async move {
        let mut out: Vec<Map<String, Json>> = rows
            .iter()
            .map(|row| {
                selection
                    .fields
                    .iter()
                    .map(|name| (to_camel_case(name), row.get(name).map_or(Json::Null, to_json)))
                    .collect()
            })
            .collect();
        for (rel_name, nested) in &selection.relationships {
            let rel = resource.relationship(rel_name).expect("parsed against the resource");
            let dest = (rel.destination)();
            let groups = ash_core::load_related_query(ctx, resource, rel_name, &rows, &nested.query).await?;
            let sizes: Vec<usize> = groups.iter().map(Vec::len).collect();
            let mut rendered = render(ctx, dest, groups.into_iter().flatten().collect(), &nested.selection).await?.into_iter();
            let many = matches!(rel.kind, RelKind::HasMany | RelKind::ManyToMany);
            for (object, size) in out.iter_mut().zip(sizes) {
                let related: Vec<Json> = rendered.by_ref().take(size).collect();
                let value = if many { Json::Array(related) } else { related.into_iter().next().unwrap_or(Json::Null) };
                object.insert(to_camel_case(rel_name), value);
            }
        }
        Ok(out.into_iter().map(Json::Object).collect())
    })
}

/// A value as JSON, as AshTypescript formats it.
fn to_json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Int(i) => Json::from(*i),
        Value::String(s) => Json::String(s.clone()),
        Value::Uuid(u) => Json::String(u.to_string()),
        Value::Map(_) | Value::Array(_) => value.to_plain_json(),
    }
}

/// `requesterEmail` → `requester_email`.
fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A filter or input as the client sent it, its keys in snake_case.
fn snake_keys(json: &Json) -> Json {
    match json {
        Json::Object(map) => Json::Object(map.iter().map(|(k, v)| (snake(k), snake_keys(v))).collect()),
        Json::Array(items) => Json::Array(items.iter().map(snake_keys).collect()),
        other => other.clone(),
    }
}

/// `"-insertedAt,id"` → `"-inserted_at,id"`.
fn snake_sort(text: &str) -> String {
    text.split(',').map(|part| snake(part.trim())).collect::<Vec<_>>().join(",")
}

/// An error, as AshTypescript reports one.
#[derive(Debug)]
pub struct Failure {
    kind: &'static str,
    message: String,
    short: &'static str,
    vars: Json,
    fields: Vec<String>,
}

impl Failure {
    fn new(kind: &'static str, short: &'static str, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), short, vars: json!({}), fields: Vec::new() }
    }

    fn action_not_found(name: &str) -> Self {
        Self { vars: json!({ "actionName": name }), ..Self::new("action_not_found", "Action not found", "RPC action %{actionName} not found") }
    }

    fn unknown_field(name: &str, resource: &ResourceDef) -> Self {
        Self {
            vars: json!({ "field": name, "resource": resource.name }),
            fields: vec![name.to_string()],
            ..Self::new("unknown_field", "Unknown field", "Unknown field %{field} for resource %{resource}")
        }
    }

    fn requires_field_selection(name: &str) -> Self {
        Self {
            vars: json!({ "field": name, "fieldType": "Relationship" }),
            fields: vec![name.to_string()],
            ..Self::new("requires_field_selection", "Field selection required", "%{fieldType} %{field} requires field selection")
        }
    }

    fn to_json(&self) -> Json {
        json!({
            "type": self.kind, "message": self.message, "shortMessage": self.short,
            "vars": self.vars, "fields": self.fields, "path": [],
        })
    }
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        match &err {
            Error::NotFound => Self::new("not_found", "Not found", "record not found"),
            Error::Forbidden => Self::new("forbidden", "Forbidden", "forbidden"),
            Error::TenantRequired { .. } => Self::new("tenant_required", "Tenant required", err.to_string()),
            Error::StaleRecord { .. } => Self::new("stale_record", "Stale record", err.to_string()),
            Error::Validation { field, message } => Self {
                fields: vec![to_camel_case(field)],
                vars: json!({ "field": field }),
                ..Self::new("invalid_attribute", "Invalid attribute", message.clone())
            },
            Error::Missing { field } => Self {
                fields: vec![to_camel_case(field)],
                vars: json!({ "field": field }),
                ..Self::new("required", "Required", "is required")
            },
            Error::NotAccepted { field, .. } => Self {
                fields: vec![to_camel_case(field)],
                ..Self::new("invalid_argument", "Invalid argument", err.to_string())
            },
            Error::Invalid(message) => Self::new("invalid", "Invalid", message.clone()),
            _ => Self::new("internal_error", "Internal error", "Something went wrong"),
        }
    }
}

#[cfg(feature = "axum")]
pub mod axum {
    //! `POST /rpc/run`, each request running as the context `context` makes of its
    //! headers: its actor and tenant. AshTypescript's `/rpc/validate`, which validates
    //! input without running the action, isn't served yet.

    use std::sync::Arc;

    use ash_core::{Context, TransactionSupport};
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::post;
    use axum::{Json, Router};

    use super::Rpc;

    /// The context a request runs in, made of its headers.
    type MakeContext<D> = Arc<dyn Fn(&HeaderMap) -> Context<D> + Send + Sync>;

    struct Served<D> {
        rpc: Rpc<D>,
        context: MakeContext<D>,
    }

    /// The RPC endpoints for `rpc`, each request in the context `context` makes of its
    /// headers.
    pub fn rpc_router<D: TransactionSupport + 'static>(
        rpc: Rpc<D>,
        context: impl Fn(&HeaderMap) -> Context<D> + Send + Sync + 'static,
    ) -> Router {
        let served = Arc::new(Served { rpc, context: Arc::new(context) });
        Router::new()
            .route("/rpc/run", post(run::<D>))
            .with_state(served)
    }

    async fn run<D: TransactionSupport + 'static>(
        State(served): State<Arc<Served<D>>>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        let ctx = (served.context)(&headers);
        Json(served.rpc.run(&ctx, &request).await)
    }
}
