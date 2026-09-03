use ash_core::compile_read_filter;
use ash_core::redact_fields;
use ash_core::{ActionDef, ActionKind, CompiledQuery, Context, DataLayer, Filter, ResourceDef, Value};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::sort::{parse_resource_sort, resource_sort_input_name};

static FALLBACK_READ: ActionDef = ActionDef::read("read");

/// Returns pluralized name for list queries (e.g. "Ticket" -> "listTickets").
pub fn list_query_name(resource_name: &str) -> String {
    let suffix = if resource_name.ends_with('s')
        || resource_name.ends_with('x')
        || resource_name.ends_with('z')
        || resource_name.ends_with("ch")
        || resource_name.ends_with("sh")
    {
        "es"
    } else {
        "s"
    };
    format!("list{}{}", resource_name, suffix)
}

/// Generates getter query name (e.g. "Ticket" -> "getTicket").
pub fn get_query_name(resource_name: &str) -> String {
    format!("get{}", resource_name)
}

/// Generates top-level read queries (`get<Resource>` and `list<Resource>s`) for a [`ResourceDef`].
pub fn build_resource_queries<D: DataLayer + Clone + 'static>(
    resource: &'static ResourceDef,
) -> (Field, Field) {
    let pk_name = resource
        .attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id");

    let read_action = resource
        .primary_read()
        .or_else(|| resource.actions.iter().find(|a| a.kind == ActionKind::Read))
        .unwrap_or(&FALLBACK_READ);

    // 1. get<Resource>(id: ID!): <Resource>
    let get_field = Field::new(
        get_query_name(resource.name),
        TypeRef::named(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ctx_ash = ctx.data::<Context<D>>()?;
                let id_arg = ctx
                    .args
                    .get("id")
                    .ok_or_else(|| async_graphql::Error::new("Missing required id argument"))?;
                let id_str = id_arg.string()?;
                let id = Uuid::parse_str(id_str)
                    .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;

                let policy_filter = compile_read_filter(resource, read_action, ctx_ash.actor.as_ref())
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                let user_filter = Filter::eq(pk_name, Value::Uuid(id));
                let combined_filter = match policy_filter {
                    Some(pf) => Filter::And(vec![pf, user_filter]),
                    None => user_filter,
                };

                let query = CompiledQuery {
                    filter: Some(combined_filter),
                    sort: Vec::new(),
                    calculations: Vec::new(),
                    aggregates: Vec::new(),
                    limit: Some(1),
                    offset: None,
                    tenant: ctx_ash.tenant.clone(),
                };

                let records = ctx_ash
                    .data
                    .run_query(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                if let Some(mut record) = records.into_iter().next() {
                    let _ = redact_fields(resource, ctx_ash.actor.as_ref(), &mut record);
                    Ok(Some(FieldValue::owned_any(record)))
                } else {
                    Ok(None)
                }
            })
        },
    )
    .argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));

    // 2. list<Resource>s(filter: ..., sort: ..., limit: Int, offset: Int): [<Resource>!]!
    let filter_input_name = resource_filter_input_name(resource.name);
    let sort_input_name = resource_sort_input_name(resource.name);

    let list_field = Field::new(
        list_query_name(resource.name),
        TypeRef::named_nn_list_nn(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ctx_ash = ctx.data::<Context<D>>()?;

                // Parse user filter if provided
                let user_filter = if let Some(filter_arg) = ctx.args.get("filter") {
                    let obj = filter_arg.object()?;
                    Some(parse_resource_filter(resource, &obj)?)
                } else {
                    None
                };

                // Compile policy filter
                let policy_filter = compile_read_filter(resource, read_action, ctx_ash.actor.as_ref())
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                let combined_filter = match (policy_filter, user_filter) {
                    (Some(pf), Some(uf)) => Some(Filter::And(vec![pf, uf])),
                    (Some(pf), None) => Some(pf),
                    (None, Some(uf)) => Some(uf),
                    (None, None) => None,
                };

                // Parse sort if provided
                let sort = if let Some(sort_arg) = ctx.args.get("sort") {
                    let list = sort_arg.list()?;
                    parse_resource_sort(resource, &list)?
                } else {
                    Vec::new()
                };

                let limit = if let Some(limit_arg) = ctx.args.get("limit") {
                    Some(limit_arg.i64()? as usize)
                } else {
                    None
                };

                let offset = if let Some(offset_arg) = ctx.args.get("offset") {
                    Some(offset_arg.i64()? as usize)
                } else {
                    None
                };

                let query = CompiledQuery {
                    filter: combined_filter,
                    sort,
                    calculations: Vec::new(),
                    aggregates: Vec::new(),
                    limit,
                    offset,
                    tenant: ctx_ash.tenant.clone(),
                };

                let mut records = ctx_ash
                    .data
                    .run_query(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                for record in &mut records {
                    let _ = redact_fields(resource, ctx_ash.actor.as_ref(), record);
                }

                Ok(Some(FieldValue::list(
                    records.into_iter().map(FieldValue::owned_any),
                )))
            })
        },
    )
    .argument(InputValue::new("filter", TypeRef::named(filter_input_name)))
    .argument(InputValue::new(
        "sort",
        TypeRef::named_list(sort_input_name),
    ))
    .argument(InputValue::new("limit", TypeRef::named(TypeRef::INT)))
    .argument(InputValue::new("offset", TypeRef::named(TypeRef::INT)));

    (get_field, list_field)
}
