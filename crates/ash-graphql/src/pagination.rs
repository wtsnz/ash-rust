use ash_core::{
    CompiledQuery, DataLayer, FieldMap, Filter, KeysetCursor, ResourceDef, Sort, Value,
    redact_fields, scope_read,
};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::request::request_context;
use crate::sort::{parse_resource_sort, resource_sort_input_name};

#[derive(Clone)]
pub struct ConnectionPayload {
    pub edges: Vec<EdgePayload>,
    pub page_info: PageInfoPayload,
    pub total_count: usize,
}

#[derive(Clone)]
pub struct EdgePayload {
    pub node: FieldMap,
    pub cursor: String,
}

#[derive(Clone)]
pub struct PageInfoPayload {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

/// Generates `<Resource>Connection` type name.
pub fn resource_connection_type_name(resource_name: &str) -> String {
    format!("{}Connection", resource_name)
}

/// Generates `<Resource>Edge` type name.
pub fn resource_edge_type_name(resource_name: &str) -> String {
    format!("{}Edge", resource_name)
}

/// Generates connection query field name (e.g. "ticketsConnection").
pub fn resource_connection_field_name(resource_name: &str) -> String {
    let lower_first = {
        let mut chars = resource_name.chars();
        match chars.next() {
            None => String::new(),
            Some(c) => c.to_lowercase().collect::<String>() + chars.as_str(),
        }
    };
    let suffix = if lower_first.ends_with('s')
        || lower_first.ends_with('x')
        || lower_first.ends_with('z')
        || lower_first.ends_with("ch")
        || lower_first.ends_with("sh")
    {
        "es"
    } else {
        "s"
    };
    format!("{}{suffix}Connection", lower_first)
}

/// Registers the shared `PageInfo` object type.
pub fn register_page_info(builder: SchemaBuilder) -> SchemaBuilder {
    let page_info = Object::new("PageInfo")
        .field(Field::new(
            "hasNextPage",
            TypeRef::named_nn(TypeRef::BOOLEAN),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(info) = ctx.parent_value.downcast_ref::<PageInfoPayload>() {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(
                            info.has_next_page,
                        ))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(false))))
                    }
                })
            },
        ))
        .field(Field::new(
            "hasPreviousPage",
            TypeRef::named_nn(TypeRef::BOOLEAN),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(info) = ctx.parent_value.downcast_ref::<PageInfoPayload>() {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(
                            info.has_previous_page,
                        ))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::Boolean(false))))
                    }
                })
            },
        ))
        .field(Field::new(
            "startCursor",
            TypeRef::named(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(info) = ctx.parent_value.downcast_ref::<PageInfoPayload>() {
                        match &info.start_cursor {
                            Some(c) => Ok(Some(FieldValue::value(GqlValue::String(c.clone())))),
                            None => Ok(None),
                        }
                    } else {
                        Ok(None)
                    }
                })
            },
        ))
        .field(Field::new(
            "endCursor",
            TypeRef::named(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(info) = ctx.parent_value.downcast_ref::<PageInfoPayload>() {
                        match &info.end_cursor {
                            Some(c) => Ok(Some(FieldValue::value(GqlValue::String(c.clone())))),
                            None => Ok(None),
                        }
                    } else {
                        Ok(None)
                    }
                })
            },
        ));

    builder.register(page_info)
}

/// Registers `<Resource>Edge` and `<Resource>Connection` types for a resource.
pub fn register_resource_connection_types(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let edge_type_name = resource_edge_type_name(resource.name);
    let conn_type_name = resource_connection_type_name(resource.name);

    // 1. Edge object
    let edge_obj = Object::new(edge_type_name.clone())
        .field(Field::new(
            "node",
            TypeRef::named_nn(resource.name),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(edge) = ctx.parent_value.downcast_ref::<EdgePayload>() {
                        Ok(Some(FieldValue::owned_any(edge.node.clone())))
                    } else {
                        Ok(None)
                    }
                })
            },
        ))
        .field(Field::new(
            "cursor",
            TypeRef::named_nn(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(edge) = ctx.parent_value.downcast_ref::<EdgePayload>() {
                        Ok(Some(FieldValue::value(GqlValue::String(
                            edge.cursor.clone(),
                        ))))
                    } else {
                        Ok(None)
                    }
                })
            },
        ));
    builder = builder.register(edge_obj);

    // 2. Connection object
    let conn_obj = Object::new(conn_type_name)
        .field(Field::new(
            "edges",
            TypeRef::named_nn_list_nn(edge_type_name),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(conn) = ctx.parent_value.downcast_ref::<ConnectionPayload>() {
                        Ok(Some(FieldValue::list(
                            conn.edges.iter().cloned().map(FieldValue::owned_any),
                        )))
                    } else {
                        Ok(Some(FieldValue::list(Vec::<FieldValue>::new())))
                    }
                })
            },
        ))
        .field(Field::new(
            "pageInfo",
            TypeRef::named_nn("PageInfo"),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(conn) = ctx.parent_value.downcast_ref::<ConnectionPayload>() {
                        Ok(Some(FieldValue::owned_any(conn.page_info.clone())))
                    } else {
                        Ok(None)
                    }
                })
            },
        ))
        .field(Field::new(
            "totalCount",
            TypeRef::named_nn(TypeRef::INT),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(conn) = ctx.parent_value.downcast_ref::<ConnectionPayload>() {
                        Ok(Some(FieldValue::value(GqlValue::Number(
                            (conn.total_count as i64).into(),
                        ))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::Number(0.into()))))
                    }
                })
            },
        ));
    builder = builder.register(conn_obj);

    builder
}

/// Builds the top-level Relay connection query field for a resource.
pub fn build_resource_connection_query<D: DataLayer + Clone + 'static>(
    resource: &'static ResourceDef,
) -> Field {
    let field_name = resource_connection_field_name(resource.name);
    let conn_type_name = resource_connection_type_name(resource.name);
    let filter_input_name = resource_filter_input_name(resource.name);
    let sort_input_name = resource_sort_input_name(resource.name);

    let pk_name = resource
        .attributes
        .iter()
        .find(|a| a.primary_key)
        .map(|a| a.name)
        .unwrap_or("id");

    let read_action = resource.default_read();

    Field::new(field_name, TypeRef::named_nn(conn_type_name), move |ctx| {
        FieldFuture::new(async move {
            let ash = request_context::<D>(&ctx)?;

            let filter = match ctx.args.get("filter").map(|f| f.object()) {
                Some(Ok(obj)) => Some(parse_resource_filter(resource, &obj)?),
                _ => None,
            };
            let sort = match ctx.args.get("sort").map(|s| s.list()) {
                Some(Ok(list)) => parse_resource_sort(resource, &list)?,
                _ => Vec::new(),
            };

            let first = ctx
                .args
                .get("first")
                .and_then(|v| v.i64().ok())
                .map(|n| n as usize);
            let last = ctx
                .args
                .get("last")
                .and_then(|v| v.i64().ok())
                .map(|n| n as usize);
            let after = ctx.args.get("after").and_then(|v| v.string().ok());
            let before = ctx.args.get("before").and_then(|v| v.string().ok());

            let limit = first.or(last).unwrap_or(20);
            let is_before = before.is_some() && after.is_none();
            let target_cursor_str = if is_before { before } else { after };

            // The connection is the primary read's rows: its preparations, the actor's
            // policies and the request's tenant. Pages are cut from it by cursor.
            let scoped = scope_read(
                resource,
                read_action,
                ash.actor.as_ref(),
                &FieldMap::new(),
                CompiledQuery {
                    filter,
                    sort,
                    limit: Some(limit + 1),
                    tenant: ash.tenant.clone(),
                    ..CompiledQuery::default()
                },
            )
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;

            // The primary key breaks ties so every row has a distinct cursor.
            let mut sort = scoped.sort.clone();
            if sort.is_empty() {
                sort.push(Sort {
                    field: pk_name.to_string(),
                    descending: false,
                });
            } else if !sort.iter().any(|s| s.field == pk_name) {
                let last_desc = sort.last().map(|s| s.descending).unwrap_or(false);
                sort.push(Sort {
                    field: pk_name.to_string(),
                    descending: last_desc,
                });
            }

            let count_query = CompiledQuery {
                sort: Vec::new(),
                limit: None,
                offset: None,
                ..scoped.clone()
            };
            let total_count = ash
                .data
                .run_query(resource, &count_query)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?
                .len();

            let mut page_filter = scoped.filter.clone();
            if let Some(c_str) = target_cursor_str
                && let Some(mut cursor) = KeysetCursor::decode(c_str)
            {
                if cursor.values.is_empty() {
                    cursor.values.push((pk_name.to_string(), Value::Uuid(cursor.id)));
                }

                let mut sort_tuples = Vec::new();
                for s in &sort {
                    let val = cursor
                        .values
                        .iter()
                        .find(|(k, _)| k == &s.field)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null);
                    sort_tuples.push((s.field.clone(), val, s.descending));
                }

                if let Some(keyset_filter) = build_keyset_filter(&sort_tuples, !is_before) {
                    page_filter = Some(match page_filter {
                        Some(f) => Filter::And(vec![f, keyset_filter]),
                        None => keyset_filter,
                    });
                }
            }

            // Backward pages read in reverse, then flip.
            let mut query_sort = sort.clone();
            if is_before {
                for s in &mut query_sort {
                    s.descending = !s.descending;
                }
            }

            let query = CompiledQuery {
                filter: page_filter,
                sort: query_sort,
                ..scoped
            };
            let mut records = ash
                .data
                .run_query(resource, &query)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;

            let has_more = records.len() > limit;
            if has_more {
                records.truncate(limit);
            }
            if is_before {
                records.reverse();
            }

            // Redact fields
            for r in &mut records {
                let _ = redact_fields(resource, ash.actor.as_ref(), r);
            }

            let mut edges = Vec::with_capacity(records.len());
            for r in records {
                let cursor_str = cursor_for_field_map(&r, &sort, pk_name);
                edges.push(EdgePayload {
                    node: r,
                    cursor: cursor_str,
                });
            }

            let start_cursor = edges.first().map(|e| e.cursor.clone());
            let end_cursor = edges.last().map(|e| e.cursor.clone());

            let has_next_page = if is_before {
                target_cursor_str.is_some()
            } else {
                has_more
            };

            let has_previous_page = if is_before {
                has_more
            } else {
                after.is_some()
            };

            let page_info = PageInfoPayload {
                has_next_page,
                has_previous_page,
                start_cursor,
                end_cursor,
            };

            Ok(Some(FieldValue::owned_any(ConnectionPayload {
                edges,
                page_info,
                total_count,
            })))
        })
    })
    .argument(InputValue::new("filter", TypeRef::named(filter_input_name)))
    .argument(InputValue::new(
        "sort",
        TypeRef::named_list(sort_input_name),
    ))
    .argument(InputValue::new("first", TypeRef::named(TypeRef::INT)))
    .argument(InputValue::new("after", TypeRef::named(TypeRef::STRING)))
    .argument(InputValue::new("last", TypeRef::named(TypeRef::INT)))
    .argument(InputValue::new("before", TypeRef::named(TypeRef::STRING)))
}

fn cursor_for_field_map(map: &FieldMap, sort: &[Sort], pk_name: &str) -> String {
    let id = map.get(pk_name).and_then(|v| v.as_uuid()).unwrap_or_default();
    let mut values = Vec::with_capacity(sort.len());
    for s in sort {
        let val = map.get(&s.field).cloned().unwrap_or(Value::Null);
        values.push((s.field.clone(), val));
    }
    KeysetCursor { id, values }.encode()
}

fn build_keyset_filter(sorts: &[(String, Value, bool)], is_after: bool) -> Option<Filter> {
    if sorts.is_empty() {
        return None;
    }
    let (field, val, desc) = &sorts[0];
    let cond = if is_after {
        if !desc {
            Filter::Gt(field.clone(), val.clone())
        } else {
            Filter::Lt(field.clone(), val.clone())
        }
    } else if !desc {
        Filter::Lt(field.clone(), val.clone())
    } else {
        Filter::Gt(field.clone(), val.clone())
    };

    if sorts.len() == 1 {
        Some(cond)
    } else {
        let eq = Filter::Eq(field.clone(), val.clone());
        let rest = build_keyset_filter(&sorts[1..], is_after)?;
        Some(Filter::Or(vec![cond, Filter::And(vec![eq, rest])]))
    }
}
