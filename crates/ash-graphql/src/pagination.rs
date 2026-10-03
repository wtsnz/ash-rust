//! Keyset pages, as AshGraphql returns a paginated read: `KeysetPageOf<Resource> { count,
//! results, startKeyset, endKeyset }`, paged with `first` / `after` and `last` / `before`.

use ash_core::{
    ActionDef, CompiledQuery, DataLayer, FieldMap, Filter, KeysetCursor, ResourceDef, Sort, Value,
    build_keyset_filter, keyset_sort, scope_read,
};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::redact::redact_record;
use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::names::camel;
use crate::preload::{Load, preload, selected};
use crate::request::request_context;
use crate::sort::{parse_resource_sort, resource_sort_input_name};
use crate::types::{attr_type_to_type_ref, parse_input_val};

/// The most records a page holds, as Ash's default `max_page_size`.
pub const MAX_PAGE_SIZE: usize = 250;

/// A page of records and the keysets at its ends.
#[derive(Clone)]
pub struct KeysetPage {
    pub results: Vec<FieldMap>,
    pub start_keyset: Option<String>,
    pub end_keyset: Option<String>,
    pub count: Option<usize>,
}

/// `Trip` → `KeysetPageOfTrip`.
pub fn keyset_page_type_name(resource_name: &str) -> String {
    format!("KeysetPageOf{resource_name}")
}

/// Registers `KeysetPageOf<Resource>`. Its fields borrow the page, so a page of records
/// is never copied to resolve it.
pub fn register_keyset_page(builder: SchemaBuilder, resource: &'static ResourceDef) -> SchemaBuilder {
    fn page<'a>(ctx: &ResolverContext<'a>) -> Option<&'a KeysetPage> {
        ctx.parent_value.downcast_ref::<KeysetPage>()
    }
    fn keyset(value: &Option<String>) -> Option<FieldValue<'static>> {
        value.as_ref().map(|v| FieldValue::value(GqlValue::String(v.clone())))
    }
    builder.register(
        Object::new(keyset_page_type_name(resource.name))
            .field(Field::new("count", TypeRef::named(TypeRef::INT), |ctx| {
                FieldFuture::new(async move {
                    Ok(page(&ctx)
                        .and_then(|p| p.count)
                        .map(|n| FieldValue::value(GqlValue::Number((n as i64).into()))))
                })
            }))
            .field(Field::new("results", TypeRef::named_nn_list(resource.name), |ctx| {
                FieldFuture::new(async move {
                    Ok(page(&ctx).map(|p| FieldValue::list(p.results.iter().map(|r| FieldValue::borrowed_any(r)))))
                })
            }))
            .field(Field::new("startKeyset", TypeRef::named(TypeRef::STRING), |ctx| {
                FieldFuture::new(async move { Ok(page(&ctx).and_then(|p| keyset(&p.start_keyset))) })
            }))
            .field(Field::new("endKeyset", TypeRef::named(TypeRef::STRING), |ctx| {
                FieldFuture::new(async move { Ok(page(&ctx).and_then(|p| keyset(&p.end_keyset))) })
            })),
    )
}

/// The action arguments a read query takes, parsed.
pub(crate) fn read_arguments(
    ctx: &ResolverContext<'_>,
    action: &ActionDef,
) -> async_graphql::Result<FieldMap> {
    let mut arguments = FieldMap::new();
    for arg in action.arguments {
        if let Some(value) = ctx.args.get(&camel(arg.name))
            && !value.is_null()
        {
            let value = parse_input_val(&value, arg.ty)?;
            if !value.is_null() {
                arguments.insert(arg.name.to_string(), value);
            }
        }
    }
    Ok(arguments)
}

/// Calculations take the action arguments named like theirs.
pub(crate) fn calculation_arguments(
    resource: &ResourceDef,
    arguments: &FieldMap,
) -> std::collections::HashMap<String, FieldMap> {
    let mut out = std::collections::HashMap::new();
    for calc in resource.calculations {
        let matched: FieldMap = calc
            .arguments
            .iter()
            .filter_map(|arg| arguments.get(arg.name).map(|val| (arg.name.to_string(), val.clone())))
            .collect();
        if !matched.is_empty() {
            out.insert(calc.name.to_string(), matched);
        }
    }
    out
}

/// Adds a read action's arguments, and `sort` and `filter`, to a query field.
pub(crate) fn read_field_arguments(
    mut field: Field,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    field = field
        .argument(InputValue::new(
            "sort",
            TypeRef::named_list(resource_sort_input_name(resource.name)),
        ))
        .argument(InputValue::new(
            "filter",
            TypeRef::named(resource_filter_input_name(resource.name)),
        ));
    for arg in action.arguments {
        let type_ref = attr_type_to_type_ref(resource.name, arg.name, arg.ty, arg.allow_nil);
        field = field.argument(InputValue::new(camel(arg.name), type_ref));
    }
    field
}

/// A keyset-paginated read through `action`: `<name>(sort, filter, first, before, after,
/// last, <arguments>): KeysetPageOf<Resource>`. Every read pages, as AshGraphql's do:
/// given no `first` or `last`, a page holds as many records as it may (250).
pub fn build_keyset_query<D: DataLayer + Clone + 'static>(
    name: String,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
) -> Field {
    let pk_name = resource
        .attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id");

    let field = Field::new(name, TypeRef::named(keyset_page_type_name(resource.name)), move |ctx| {
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
            let number = |name: &str| {
                ctx.args.get(name).filter(|v| !v.is_null()).map(|v| v.i64()).transpose()
            };
            let keyset = |name: &str| ctx.args.get(name).and_then(|v| v.string().ok().map(str::to_string));
            let (first, last) = (number("first")?, number("last")?);
            let (after, before) = (keyset("after"), keyset("before"));
            let limit = page_limit(first, last, after.is_some(), before.is_some())?;
            let backward = last.is_some() || (before.is_some() && after.is_none());

            let scoped = scope_read(
                resource,
                action,
                ash.actor.as_ref(),
                &arguments,
                CompiledQuery {
                    filter,
                    sort,
                    calculation_args: calculation_arguments(resource, &arguments),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                },
            )
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

            // A page sorts stably, so every record has its own keyset.
            let sort = keyset_sort(resource, scoped.sort.clone());

            let count = if ctx.look_ahead().field("count").exists() {
                let query = CompiledQuery {
                    sort: Vec::new(),
                    limit: None,
                    offset: None,
                    ..scoped.clone()
                };
                let count = ash
                    .data
                    .count(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                Some(count)
            } else {
                None
            };

            let mut page_filter = scoped.filter.clone();
            let cursor = if backward { before.as_deref() } else { after.as_deref() };
            if let Some(cursor) = cursor.and_then(KeysetCursor::decode) {
                let tuples: Vec<(String, Value, bool)> = sort
                    .iter()
                    .map(|s| {
                        let value = cursor
                            .values
                            .iter()
                            .find(|(k, _)| k == &s.field)
                            .map(|(_, v)| v.clone())
                            .unwrap_or_else(|| {
                                if s.field == pk_name { Value::Uuid(cursor.id) } else { Value::Null }
                            });
                        (s.field.clone(), value, s.descending)
                    })
                    .collect();
                if let Some(keyset_filter) = build_keyset_filter(&tuples, !backward) {
                    page_filter = Some(match page_filter {
                        Some(f) => Filter::And(vec![f, keyset_filter]),
                        None => keyset_filter,
                    });
                }
            }

            // A backward page reads in reverse, then flips.
            let mut query_sort = sort.clone();
            if backward {
                for s in &mut query_sort {
                    s.descending = !s.descending;
                }
            }
            // Only what's selected, and the sort a keyset holds.
            let load = Load::of(
                resource,
                &selected(ctx.ctx.field(), Some("results")),
                sort.iter().map(|s| s.field.as_str()),
            );
            let query = load.onto(CompiledQuery {
                filter: page_filter,
                sort: query_sort,
                limit: Some(limit),
                offset: None,
                ..scoped
            });
            let mut records = ash
                .data
                .run_query(resource, &query)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            if backward {
                records.reverse();
            }
            for record in &mut records {
                redact_record(resource, ash.actor.as_ref(), record);
            }
            preload(&ash, resource, selected(ctx.ctx.field(), Some("results")), &mut records).await?;
            let keyset = |record: Option<&FieldMap>| record.map(|r| keyset_of(r, &sort, pk_name));
            Ok(Some(FieldValue::owned_any(KeysetPage {
                start_keyset: keyset(records.first()),
                end_keyset: keyset(records.last()),
                results: records,
                count,
            })))
        })
    });

    read_field_arguments(field, resource, action)
        .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)))
        .argument(InputValue::new("before", TypeRef::named(TypeRef::STRING)))
        .argument(InputValue::new("after", TypeRef::named(TypeRef::STRING)))
        .argument(InputValue::new("last", TypeRef::named(TypeRef::INT)))
}

/// How many records a page holds, as AshGraphql validates a keyset read's arguments:
/// `first` records after `after`, `last` before `before`, or, given neither, as many as a
/// page may hold. Never more than that.
fn page_limit(first: Option<i64>, last: Option<i64>, after: bool, before: bool) -> async_graphql::Result<usize> {
    let error = |message: &str| Err(async_graphql::Error::new(message));
    let cursors = "You can pass either `first` and `after` cursor, or `last` and `before` cursor";
    match (first, last) {
        (Some(_), Some(_)) => error("You can pass either `first` or `last`, not both"),
        (Some(_), _) if before => error(cursors),
        (_, Some(_)) if after => error(cursors),
        (Some(n), _) if n < 1 => error("`first` must be a positive integer"),
        (_, Some(n)) if n < 1 => error("`last` must be a positive integer"),
        (Some(n), _) => Ok((n as usize).min(MAX_PAGE_SIZE)),
        (_, Some(n)) if before => Ok((n as usize).min(MAX_PAGE_SIZE)),
        (_, Some(_)) => error("You can pass `last` only with `before` cursor"),
        (None, None) => Ok(MAX_PAGE_SIZE),
    }
}

fn keyset_of(record: &FieldMap, sort: &[Sort], pk_name: &str) -> String {
    let id = record.get(pk_name).and_then(|v| v.as_uuid()).unwrap_or_default();
    let values = sort
        .iter()
        .map(|s| (s.field.clone(), record.get(&s.field).cloned().unwrap_or(Value::Null)))
        .collect();
    KeysetCursor { id, values }.encode()
}
