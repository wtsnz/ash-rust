//! Keyset pages, as AshGraphql returns a paginated read: `KeysetPageOf<Resource> { count,
//! results, startKeyset, endKeyset }`, paged with `first` / `after` and `last` / `before`.

use ash_core::{
    ActionDef, CompiledQuery, DataLayer, FieldMap, Filter, KeysetCursor, ResourceDef, Sort, Value,
    redact_fields, scope_read,
};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::names::camel;
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

/// Registers `KeysetPageOf<Resource>`.
pub fn register_keyset_page(builder: SchemaBuilder, resource: &'static ResourceDef) -> SchemaBuilder {
    let page = |ctx: &ResolverContext<'_>| ctx.parent_value.downcast_ref::<KeysetPage>().cloned();
    let text = |value: Option<String>| value.map(|v| FieldValue::value(GqlValue::String(v)));
    builder.register(
        Object::new(keyset_page_type_name(resource.name))
            .field(Field::new("count", TypeRef::named(TypeRef::INT), move |ctx| {
                FieldFuture::new(async move {
                    Ok(page(&ctx)
                        .and_then(|p| p.count)
                        .map(|n| FieldValue::value(GqlValue::Number((n as i64).into()))))
                })
            }))
            .field(Field::new(
                "results",
                TypeRef::named_nn_list(resource.name),
                move |ctx| {
                    FieldFuture::new(async move {
                        Ok(page(&ctx).map(|p| FieldValue::list(p.results.into_iter().map(FieldValue::owned_any))))
                    })
                },
            ))
            .field(Field::new("startKeyset", TypeRef::named(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move { Ok(text(page(&ctx).and_then(|p| p.start_keyset))) })
            }))
            .field(Field::new("endKeyset", TypeRef::named(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move { Ok(text(page(&ctx).and_then(|p| p.end_keyset))) })
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
/// last, <arguments>): KeysetPageOf<Resource>`. Without `first` or `last` the page holds
/// every record, as an Ash read whose pagination isn't required does.
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
                Some(filter) => Some(parse_resource_filter(resource, &filter.object()?)?),
                None => None,
            };
            let sort = match ctx.args.get("sort").filter(|value| !value.is_null()) {
                Some(sort) => parse_resource_sort(resource, &sort.list()?)?,
                None => Vec::new(),
            };
            let number = |name: &str| {
                ctx.args
                    .get(name)
                    .and_then(|v| v.i64().ok())
                    .map(|n| (n.max(0) as usize).min(MAX_PAGE_SIZE))
            };
            let keyset = |name: &str| ctx.args.get(name).and_then(|v| v.string().ok().map(str::to_string));
            let (first, last) = (number("first"), number("last"));
            let (after, before) = (keyset("after"), keyset("before"));
            let backward = before.is_some() && after.is_none() || (last.is_some() && first.is_none());
            let limit = if backward { last.or(first) } else { first.or(last) };

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

            // The primary key breaks ties, so every record has its own keyset.
            let mut sort = scoped.sort.clone();
            if !sort.iter().any(|s| s.field == pk_name) {
                let descending = sort.last().is_some_and(|s| s.descending);
                sort.push(Sort {
                    field: pk_name.to_string(),
                    descending,
                });
            }

            let count = if ctx.look_ahead().field("count").exists() {
                let query = CompiledQuery {
                    sort: Vec::new(),
                    limit: None,
                    offset: None,
                    ..scoped.clone()
                };
                let rows = ash
                    .data
                    .run_query(resource, &query)
                    .await
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;
                Some(rows.len())
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
                if let Some(keyset_filter) = keyset_filter(&tuples, !backward) {
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
            let query = CompiledQuery {
                filter: page_filter,
                sort: query_sort,
                limit,
                offset: None,
                ..scoped
            };
            let mut records = ash
                .data
                .run_query(resource, &query)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            if backward {
                records.reverse();
            }
            for record in &mut records {
                let _ = redact_fields(resource, ash.actor.as_ref(), record);
            }
            Ok(Some(FieldValue::owned_any(KeysetPage {
                start_keyset: records.first().map(|r| keyset_of(r, &sort, pk_name)),
                end_keyset: records.last().map(|r| keyset_of(r, &sort, pk_name)),
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

fn keyset_of(record: &FieldMap, sort: &[Sort], pk_name: &str) -> String {
    let id = record.get(pk_name).and_then(|v| v.as_uuid()).unwrap_or_default();
    let values = sort
        .iter()
        .map(|s| (s.field.clone(), record.get(&s.field).cloned().unwrap_or(Value::Null)))
        .collect();
    KeysetCursor { id, values }.encode()
}

fn keyset_filter(sorts: &[(String, Value, bool)], after: bool) -> Option<Filter> {
    let ((field, value, descending), rest) = sorts.split_first()?;
    let beyond = if after != *descending {
        Filter::Gt(field.clone(), value.clone())
    } else {
        Filter::Lt(field.clone(), value.clone())
    };
    match keyset_filter(rest, after) {
        None => Some(beyond),
        Some(rest) => Some(Filter::Or(vec![
            beyond,
            Filter::And(vec![Filter::Eq(field.clone(), value.clone()), rest]),
        ])),
    }
}
