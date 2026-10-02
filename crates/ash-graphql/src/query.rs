use ash_core::{
    ActionDef, CompiledQuery, Context, DataLayer, FieldMap, Filter, ResourceDef, Value,
    scope_read,
};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::redact::redact_record;
use crate::filter::parse_resource_filter;
use crate::names::{camel, plural};
use crate::pagination::{build_keyset_query, calculation_arguments, read_arguments, read_field_arguments};
use crate::preload::{preload, selected};
use crate::request::request_context;
use crate::sort::parse_resource_sort;

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

/// `Ticket` → `listTickets`.
pub fn list_query_name(resource_name: &str) -> String {
    format!("list{}", plural(resource_name))
}

/// A read action's own query: `recent` on `Trip` → `recentTrips`.
pub fn list_query_name_for_action(action_name: &str, resource_name: &str) -> String {
    format!("{}{}", camel(action_name), plural(resource_name))
}

/// `Ticket` → `getTicket`.
pub fn get_query_name(resource_name: &str) -> String {
    format!("get{resource_name}")
}

/// `get<Resource>(id: ID!): <Resource>` and the keyset-paginated
/// `list<Resources>(sort, filter, first, before, after, last): KeysetPageOf<Resource>`
/// through the primary read, as AshGraphql's `get` and `list` queries are.
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
                let id = Uuid::parse_str(id_arg.string()?)
                    .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;
                // A get sees what the primary read sees: an archived record is not found.
                let query = CompiledQuery {
                    filter: Some(Filter::eq(pk_name, Value::Uuid(id))),
                    limit: Some(1),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                };
                let records = run_read(&ash, resource, read_action, &FieldMap::new(), query).await?;
                let Some(mut record) = records.into_iter().next() else {
                    return Ok(None);
                };
                redact_record(resource, ash.actor.as_ref(), &mut record);
                let fields = selected(ctx.ctx.field(), None);
                preload(&ash, resource, fields, std::slice::from_mut(&mut record)).await?;
                Ok(Some(FieldValue::owned_any(record)))
            })
        },
    )
    .argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));

    let list_field = build_keyset_query::<D>(list_query_name(resource.name), resource, read_action);
    (get_field, list_field)
}

/// A read action's own query, `<action><Resources>(sort, filter, <arguments>): [<Resource>!]!`:
/// a plain list, as AshGraphql gives a read without pagination.
pub fn build_read_action_query<D: DataLayer + Clone + 'static>(
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    let field = Field::new(
        list_query_name_for_action(action.name, resource.name),
        TypeRef::named_nn_list_nn(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ash = request_context::<D>(&ctx)?;
                let arguments = read_arguments(&ctx, action)?;
                let filter = match ctx.args.get("filter").filter(|value| !value.is_null()) {
                    Some(filter) => Some(parse_resource_filter(resource, filter.as_value())?),
                    None => None,
                };
                let sort = match ctx.args.get("sort").filter(|value| !value.is_null()) {
                    Some(sort) => parse_resource_sort(resource, sort.as_value())?,
                    None => Vec::new(),
                };
                let query = CompiledQuery {
                    filter,
                    sort,
                    calculation_args: calculation_arguments(resource, &arguments),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                };
                let mut records = run_read(&ash, resource, action, &arguments, query).await?;
                for record in &mut records {
                    redact_record(resource, ash.actor.as_ref(), record);
                }
                preload(&ash, resource, selected(ctx.ctx.field(), None), &mut records).await?;
                Ok(Some(FieldValue::list(records.into_iter().map(FieldValue::owned_any))))
            })
        },
    );
    read_field_arguments(field, resource, action)
}
