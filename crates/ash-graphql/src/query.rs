use ash_core::{
    ActionDef, CompiledQuery, Context, DataLayer, FieldMap, Filter, ResourceDef, Value,
    redact_fields, scope_read,
};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::mutation::to_camel_case;
use crate::request::request_context;
use crate::sort::{parse_resource_sort, resource_sort_input_name};
use crate::types::{attr_type_to_type_ref, parse_input_val};

/// Runs `query` as a read through `action` in `ctx`: with the action's preparations and
/// argument filters, the actor's read policies and the context's tenant.
pub(crate) async fn run_read<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    action: &ActionDef,
    arguments: &FieldMap,
    query: CompiledQuery,
) -> async_graphql::Result<Vec<FieldMap>> {
    let query = scope_read(resource, action, ctx.actor.as_ref(), arguments, query)
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    ctx.data
        .run_query(resource, &query)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))
}

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

/// Generates custom list query name for a specific read action (e.g. "byCategory" -> "byCategoryTickets").
pub fn list_query_name_for_action(action_name: &str, resource_name: &str) -> String {
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
    format!("{}{}{}", to_camel_case(action_name), resource_name, suffix)
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

    let read_action = resource.default_read();

    // 1. get<Resource>(id: ID!): <Resource>
    let get_field = Field::new(
        get_query_name(resource.name),
        TypeRef::named(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ash = request_context::<D>(&ctx)?;
                let id_arg = ctx
                    .args
                    .get("id")
                    .ok_or_else(|| async_graphql::Error::new("Missing required id argument"))?;
                let id_str = id_arg.string()?;
                let id = Uuid::parse_str(id_str)
                    .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;

                // A get sees what the primary read sees: an archived record is not found.
                let query = CompiledQuery {
                    filter: Some(Filter::eq(pk_name, Value::Uuid(id))),
                    limit: Some(1),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                };
                let records =
                    run_read(&ash, resource, read_action, &FieldMap::new(), query).await?;

                if let Some(mut record) = records.into_iter().next() {
                    let _ = redact_fields(resource, ash.actor.as_ref(), &mut record);
                    Ok(Some(FieldValue::owned_any(record)))
                } else {
                    Ok(None)
                }
            })
        },
    )
    .argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));

    // 2. list<Resource>s(filter: ..., sort: ..., limit: Int, offset: Int, <action_args>): [<Resource>!]!
    let list_field =
        build_read_field_internal::<D>(list_query_name(resource.name), read_action, resource);

    (get_field, list_field)
}

/// Builds a custom GraphQL query [`Field`] for a specific read [`ActionDef`].
pub fn build_read_action_query<D: DataLayer + Clone + 'static>(
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    let field_name = list_query_name_for_action(action.name, resource.name);
    build_read_field_internal::<D>(field_name, action, resource)
}

fn build_read_field_internal<D: DataLayer + Clone + 'static>(
    field_name: String,
    read_action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    let filter_input_name = resource_filter_input_name(resource.name);
    let sort_input_name = resource_sort_input_name(resource.name);

    let mut field = Field::new(
        field_name,
        TypeRef::named_nn_list_nn(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ash = request_context::<D>(&ctx)?;

                let mut action_args = FieldMap::new();
                for arg in read_action.arguments {
                    if let Some(val) = ctx.args.get(arg.name) {
                        let ash_val = parse_input_val(&val, arg.ty)?;
                        if !ash_val.is_null() {
                            action_args.insert(arg.name.to_string(), ash_val);
                        }
                    }
                }

                let filter = match ctx.args.get("filter").map(|f| f.object()) {
                    Some(Ok(obj)) => Some(parse_resource_filter(resource, &obj)?),
                    _ => None,
                };
                let sort = match ctx.args.get("sort").map(|s| s.list()) {
                    Some(Ok(list)) => parse_resource_sort(resource, &list)?,
                    _ => Vec::new(),
                };
                let limit = ctx
                    .args
                    .get("limit")
                    .and_then(|v| v.i64().ok())
                    .map(|n| n as usize);
                let offset = ctx
                    .args
                    .get("offset")
                    .and_then(|v| v.i64().ok())
                    .map(|n| n as usize);

                // Calculations take the action arguments named like theirs.
                let mut calculation_args = std::collections::HashMap::new();
                for calc in resource.calculations {
                    let matched: FieldMap = calc
                        .arguments
                        .iter()
                        .filter_map(|arg| {
                            action_args
                                .get(arg.name)
                                .map(|val| (arg.name.to_string(), val.clone()))
                        })
                        .collect();
                    if !matched.is_empty() {
                        calculation_args.insert(calc.name.to_string(), matched);
                    }
                }

                let query = CompiledQuery {
                    filter,
                    sort,
                    calculation_args,
                    limit,
                    offset,
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                };
                let mut records =
                    run_read(&ash, resource, read_action, &action_args, query).await?;

                for record in &mut records {
                    let _ = redact_fields(resource, ash.actor.as_ref(), record);
                }

                Ok(Some(FieldValue::list(
                    records.into_iter().map(FieldValue::owned_any),
                )))
            })
        },
    );

    // Add action-specific arguments
    for arg in read_action.arguments {
        let type_ref = attr_type_to_type_ref(resource.name, arg.name, arg.ty, arg.allow_nil);
        field = field.argument(InputValue::new(arg.name, type_ref));
    }

    field = field
        .argument(InputValue::new("filter", TypeRef::named(filter_input_name)))
        .argument(InputValue::new(
            "sort",
            TypeRef::named_list(sort_input_name),
        ))
        .argument(InputValue::new("limit", TypeRef::named(TypeRef::INT)))
        .argument(InputValue::new("offset", TypeRef::named(TypeRef::INT)));

    field
}
