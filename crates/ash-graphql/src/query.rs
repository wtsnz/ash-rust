use ash_core::{
    compile_read_filter, redact_fields, ActionDef, ActionKind, CompiledQuery, Context, DataLayer,
    FieldMap, Filter, PreparationDef, ResourceDef, Sort, Value,
};
use async_graphql::dynamic::*;
use uuid::Uuid;

use crate::filter::{parse_resource_filter, resource_filter_input_name};
use crate::mutation::to_camel_case;
use crate::sort::{parse_resource_sort, resource_sort_input_name};
use crate::types::{attr_type_to_type_ref, parse_input_val};

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
                    calculation_args: Default::default(),
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
                let ctx_ash = ctx.data::<Context<D>>()?;

                // 1. Extract action arguments
                let mut action_args = FieldMap::new();
                for arg in read_action.arguments {
                    if let Some(val) = ctx.args.get(arg.name) {
                        let ash_val = parse_input_val(&val, arg.ty)?;
                        if !ash_val.is_null() {
                            action_args.insert(arg.name.to_string(), ash_val);
                        }
                    }
                }

                // 2. Parse user filter if provided
                let user_filter = if let Some(filter_arg) = ctx.args.get("filter") {
                    if let Ok(obj) = filter_arg.object() {
                        Some(parse_resource_filter(resource, &obj)?)
                    } else {
                        None
                    }
                } else {
                    None
                };

                // 3. Compile policy filter
                let policy_filter = compile_read_filter(resource, read_action, ctx_ash.actor.as_ref())
                    .map_err(|e| async_graphql::Error::new(e.to_string()))?;

                // 4. Action preparations
                let mut prep_filter = None;
                let mut prep_sort = Vec::new();
                let mut prep_limit = None;
                let mut prep_offset = None;

                for prep in read_action.preparations {
                    match *prep {
                        PreparationDef::Filter(filter_fn) => {
                            let pf = filter_fn();
                            prep_filter = match prep_filter {
                                Some(cur) => Some(Filter::and([cur, pf])),
                                None => Some(pf),
                            };
                        }
                        PreparationDef::FilterWithArgs(filter_fn) => {
                            let pf = filter_fn(&action_args);
                            prep_filter = match prep_filter {
                                Some(cur) => Some(Filter::and([cur, pf])),
                                None => Some(pf),
                            };
                        }
                        PreparationDef::Sort { field, descending } => {
                            prep_sort.push(Sort {
                                field: field.to_string(),
                                descending,
                            });
                        }
                        PreparationDef::Limit(l) => {
                            if prep_limit.is_none() {
                                prep_limit = Some(l);
                            }
                        }
                        PreparationDef::Offset(o) => {
                            if prep_offset.is_none() {
                                prep_offset = Some(o);
                            }
                        }
                    }
                }

                // 5. If action arguments match resource attributes and weren't handled by preparation,
                // add equality filters
                let mut arg_filters = Vec::new();
                for (name, val) in &action_args {
                    if resource.attributes.iter().any(|a| a.name == name.as_str()) {
                        arg_filters.push(Filter::eq(name.as_str(), val.clone()));
                    }
                }
                if !arg_filters.is_empty() {
                    let af = if arg_filters.len() == 1 {
                        arg_filters.remove(0)
                    } else {
                        Filter::And(arg_filters)
                    };
                    prep_filter = match prep_filter {
                        Some(cur) => Some(Filter::and([cur, af])),
                        None => Some(af),
                    };
                }

                // 6. Combine all filters
                let mut all_filters = Vec::new();
                if let Some(pf) = policy_filter {
                    all_filters.push(pf);
                }
                if let Some(pf) = prep_filter {
                    all_filters.push(pf);
                }
                if let Some(uf) = user_filter {
                    all_filters.push(uf);
                }
                let combined_filter = match all_filters.len() {
                    0 => None,
                    1 => Some(all_filters.remove(0)),
                    _ => Some(Filter::And(all_filters)),
                };

                // 7. Parse sort if provided
                let sort = if let Some(sort_arg) = ctx.args.get("sort") {
                    if let Ok(list) = sort_arg.list() {
                        parse_resource_sort(resource, &list)?
                    } else if !prep_sort.is_empty() {
                        prep_sort
                    } else {
                        Vec::new()
                    }
                } else if !prep_sort.is_empty() {
                    prep_sort
                } else {
                    Vec::new()
                };

                let limit = ctx.args.get("limit").and_then(|v| v.i64().ok()).map(|n| n as usize).or(prep_limit);
                let offset = ctx.args.get("offset").and_then(|v| v.i64().ok()).map(|n| n as usize).or(prep_offset);

                // 8. Match calculation arguments if any calculations accept args
                let mut calc_args = std::collections::HashMap::new();
                for calc in resource.calculations {
                    if !calc.arguments.is_empty() {
                        let mut matched = FieldMap::new();
                        for arg_def in calc.arguments {
                            if let Some(val) = action_args.get(arg_def.name) {
                                matched.insert(arg_def.name.to_string(), val.clone());
                            }
                        }
                        if !matched.is_empty() {
                            calc_args.insert(calc.name.to_string(), matched);
                        }
                    }
                }

                let query = CompiledQuery {
                    filter: combined_filter,
                    sort,
                    calculations: Vec::new(),
                    calculation_args: calc_args,
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
