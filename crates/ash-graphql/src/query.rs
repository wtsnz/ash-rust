use ash_core::{
    ActionDef, CompiledQuery, Context, DataLayer, FieldMap, Filter, ResourceDef, Value,
    scope_read,
};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::redact::redact_record;
use crate::names::{camel, plural};
use crate::pagination::{PageStrategy, build_keyset_query, build_offset_query, read_field_arguments, scoped_read};
use crate::preload::{Load, preload, selected};
use crate::request::request_context;

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
    let mut records = ctx
        .data
        .run_query(resource, &query)
        .await
        .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    crate::pagination::after_read(action, arguments, &mut records)?;
    Ok(records)
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

/// `get<Resource>(id: ID!): <Resource>` and `list<Resources>(sort, filter, ...)` through
/// the primary read, as AshGraphql's `get` and `list` queries are: the list paged as
/// the read pages ([`build_list_query`]).
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
                let fields = selected(ctx.ctx.field(), None);
                let query = Load::of(resource, &fields, []).onto(CompiledQuery {
                    filter: Some(Filter::eq(pk_name, Value::Uuid(id))),
                    limit: Some(1),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                });
                let records = run_read(&ash, resource, read_action, &FieldMap::new(), query).await?;
                let Some(mut record) = records.into_iter().next() else {
                    return Ok(None);
                };
                redact_record(resource, ash.actor.as_ref(), &mut record);
                preload(&ash, resource, fields, std::slice::from_mut(&mut record)).await?;
                Ok(Some(FieldValue::owned_any(record)))
            })
        },
    )
    .argument(InputValue::new("id", TypeRef::named_nn(TypeRef::ID)));

    let list_field = build_list_query::<D>(list_query_name(resource.name), resource, read_action);
    (get_field, list_field)
}

/// A read action's own query, `<action><Resources>(sort, filter, <arguments>, ...)`, paged
/// as the action pages ([`build_list_query`]).
pub fn build_read_action_query<D: DataLayer + Clone + 'static>(
    action: &'static ActionDef,
    resource: &'static ResourceDef,
) -> Field {
    build_list_query::<D>(list_query_name_for_action(action.name, resource.name), resource, action)
}

/// A list query through `action`, as AshGraphql's `list` query: a `KeysetPageOf<Resource>`
/// where the action pages by keyset, a `PageOf<Resource>` where it pages by offset only,
/// and a plain `[<Resource>!]!` where it doesn't page.
pub fn build_list_query<D: DataLayer + Clone + 'static>(
    name: String,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    match PageStrategy::of(action) {
        Some(PageStrategy::Keyset) => build_keyset_query::<D>(name, resource, action),
        Some(PageStrategy::Offset) => build_offset_query::<D>(name, resource, action),
        None => build_unpaged_query::<D>(name, resource, action),
    }
}

/// `<name>(sort, filter, <arguments>): [<Resource>!]!`: every record the read finds.
fn build_unpaged_query<D: DataLayer + Clone + 'static>(
    name: String,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    let field = Field::new(
        name,
        TypeRef::named_nn_list_nn(resource.name),
        move |ctx| {
            FieldFuture::new(async move {
                let ash = request_context::<D>(&ctx)?;
                let (arguments, scoped) = scoped_read(&ctx, &ash, resource, action)?;
                let fields = selected(ctx.ctx.field(), None);
                let load = Load::of(resource, &fields, scoped.sort.iter().map(|s| s.field.as_str()));
                let query = load.onto(scoped);
                let mut records = ash
                    .data
                    .run_query(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                crate::pagination::after_read(action, &arguments, &mut records)?;
                for record in &mut records {
                    redact_record(resource, ash.actor.as_ref(), record);
                }
                preload(&ash, resource, fields, &mut records).await?;
                Ok(Some(FieldValue::list(records.into_iter().map(FieldValue::owned_any))))
            })
        },
    );
    read_field_arguments(field, resource, action)
}
