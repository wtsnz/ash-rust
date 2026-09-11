use ash_core::eval as eval_expr;
use ash_core::redact_fields;
use ash_core::{
    Actor, AttrType, CompiledQuery, Context, DataLayer, FieldMap, Filter, RelKind, ResourceDef,
    Value,
};

use crate::read_scope::scoped_read_filter;
use async_graphql::Value as GqlValue;
use async_graphql::dataloader::DataLoader;
use async_graphql::dynamic::*;

use crate::dataloader::{AshBatchLoader, BelongsToKey, HasManyKey, ManyToManyKey};
use crate::types::{
    ash_value_to_graphql_value, ash_value_to_graphql_value_typed, attr_type_to_type_ref,
    enum_type_name,
};

/// Builds the GraphQL [`Object`] definition for an Ash [`ResourceDef`].
pub fn build_resource_object<D: DataLayer + Clone + 'static>(
    resource: &'static ResourceDef,
) -> Object {
    let mut obj = Object::new(resource.name);

    // 1. Attributes
    for attr in resource.attributes {
        let attr_name = attr.name;
        let attr_ty = attr.ty;
        let has_field_policy = resource
            .field_policies
            .iter()
            .any(|fp| fp.field == attr.name);
        let allow_nil = attr.allow_nil || has_field_policy;
        let type_ref = attr_type_to_type_ref(resource.name, attr.name, attr.ty, allow_nil);

        let field = Field::new(attr_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    let actor = ctx
                        .data_opt::<Actor>()
                        .or_else(|| ctx.data_opt::<Option<Actor>>().and_then(|opt| opt.as_ref()));

                    if has_field_policy {
                        let mut check_map = map.clone();
                        let _ = redact_fields(resource, actor, &mut check_map);
                        if let Some(val) = check_map.get(attr_name) {
                            if val.is_null() {
                                return Ok(None);
                            }
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value_typed(
                                val, attr_ty,
                            ))));
                        } else {
                            return Ok(None);
                        }
                    }

                    if let Some(val) = map.get(attr_name) {
                        if val.is_null() {
                            return Ok(None);
                        }
                        return Ok(Some(FieldValue::value(ash_value_to_graphql_value_typed(
                            val, attr_ty,
                        ))));
                    }
                }
                Ok(None)
            })
        });

        obj = obj.field(field);
    }

    // 2. Calculations
    for calc in resource.calculations {
        let calc_name = calc.name;
        let calc_ty = calc.ty;
        let type_ref = attr_type_to_type_ref(resource.name, calc.name, calc.ty, true);
        let expr = calc.expr;

        let field = Field::new(calc_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    match map.get(calc_name) {
                        Some(val) if !val.is_null() => {
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value_typed(
                                val, calc_ty,
                            ))));
                        }
                        _ => {}
                    }

                    match eval_expr(&expr, map) {
                        Ok(val) if !val.is_null() => {
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value_typed(
                                &val, calc_ty,
                            ))));
                        }
                        _ => {}
                    }
                }
                Ok(None)
            })
        });

        obj = obj.field(field);
    }

    // 3. Aggregates
    for agg in resource.aggregates {
        let agg_name = agg.name;
        let type_ref = attr_type_to_type_ref(resource.name, agg.name, agg.ty, true);

        let field = Field::new(agg_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    match map.get(agg_name) {
                        Some(val) if !val.is_null() => {
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value(val))));
                        }
                        _ => {}
                    }
                }
                Ok(Some(FieldValue::value(GqlValue::Number(0.into()))))
            })
        });

        obj = obj.field(field);
    }

    // 4. Relationships (BelongsTo, HasMany, ManyToMany)
    for rel in resource.relationships {
        let dest_fn = rel.destination;
        let dest_res = dest_fn();
        let dest_name = dest_res.name;
        let rel_name = rel.name;
        let rel_kind = rel.kind;
        let source_attr = rel.source_attribute;
        let dest_attr = rel.destination_attribute;
        let through_fn = rel.through;
        let source_join = rel.source_attribute_on_join_resource;
        let dest_join = rel.destination_attribute_on_join_resource;

        let type_ref = match rel_kind {
            RelKind::BelongsTo | RelKind::HasOne => TypeRef::named(dest_name),
            RelKind::HasMany | RelKind::ManyToMany => TypeRef::named_nn_list_nn(dest_name),
        };

        let field = Field::new(rel_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    // Check if preloaded
                    if let Some(val) = map.get(rel_name) {
                        match val {
                            Value::Null => return Ok(None),
                            Value::Map(m) => {
                                let mut item = m.clone();
                                let _ = redact_fields(dest_res, ctx.data_opt::<Actor>(), &mut item);
                                return Ok(Some(FieldValue::owned_any(item)));
                            }
                            Value::Array(arr) => {
                                let items: Vec<FieldValue> = arr
                                    .iter()
                                    .filter_map(|v| match v {
                                        Value::Map(m) => {
                                            let mut item = m.clone();
                                            let _ = redact_fields(
                                                dest_res,
                                                ctx.data_opt::<Actor>(),
                                                &mut item,
                                            );
                                            Some(FieldValue::owned_any(item))
                                        }
                                        _ => None,
                                    })
                                    .collect();
                                return Ok(Some(FieldValue::list(items)));
                            }
                            _ => {}
                        }
                    }

                    // Try DataLoader
                    if let Some(loader) = ctx.data_opt::<DataLoader<AshBatchLoader<D>>>() {
                        match rel_kind {
                            RelKind::BelongsTo => {
                                if let Some(foreign_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let key = BelongsToKey {
                                        dest_resource: dest_name,
                                        dest_attr,
                                        foreign_id,
                                    };
                                    if let Ok(Some(mut record)) = loader.load_one(key).await {
                                        let _ = redact_fields(
                                            dest_res,
                                            ctx.data_opt::<Actor>(),
                                            &mut record,
                                        );
                                        return Ok(Some(FieldValue::owned_any(record)));
                                    }
                                }
                                return Ok(None);
                            }
                            RelKind::HasOne => {
                                if let Some(source_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let key = HasManyKey {
                                        dest_resource: dest_name,
                                        dest_attr,
                                        source_id,
                                    };
                                    if let Ok(Some(records)) = loader.load_one(key).await
                                        && let Some(mut r) = records.into_iter().next()
                                    {
                                        let _ = redact_fields(
                                            dest_res,
                                            ctx.data_opt::<Actor>(),
                                            &mut r,
                                        );
                                        return Ok(Some(FieldValue::owned_any(r)));
                                    }
                                }
                                return Ok(None);
                            }
                            RelKind::HasMany => {
                                if let Some(source_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let key = HasManyKey {
                                        dest_resource: dest_name,
                                        dest_attr,
                                        source_id,
                                    };
                                    if let Ok(Some(records)) = loader.load_one(key).await {
                                        let items: Vec<FieldValue> = records
                                            .into_iter()
                                            .map(|mut r| {
                                                let _ = redact_fields(
                                                    dest_res,
                                                    ctx.data_opt::<Actor>(),
                                                    &mut r,
                                                );
                                                FieldValue::owned_any(r)
                                            })
                                            .collect();
                                        return Ok(Some(FieldValue::list(items)));
                                    }
                                }
                                return Ok(Some(FieldValue::list(Vec::<FieldValue>::new())));
                            }
                            RelKind::ManyToMany => {
                                if let Some(through) = through_fn
                                    && let Some(src_join) = source_join
                                    && let Some(dst_join) = dest_join
                                    && let Some(source_id) =
                                        map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let key = ManyToManyKey {
                                        join_resource: through().name,
                                        dest_resource: dest_name,
                                        source_attr_on_join: src_join,
                                        dest_attr_on_join: dst_join,
                                        source_id,
                                    };
                                    if let Ok(Some(records)) = loader.load_one(key).await {
                                        let items: Vec<FieldValue> = records
                                            .into_iter()
                                            .map(|mut r| {
                                                let _ = redact_fields(
                                                    dest_res,
                                                    ctx.data_opt::<Actor>(),
                                                    &mut r,
                                                );
                                                FieldValue::owned_any(r)
                                            })
                                            .collect();
                                        return Ok(Some(FieldValue::list(items)));
                                    }
                                }
                                return Ok(Some(FieldValue::list(Vec::<FieldValue>::new())));
                            }
                        }
                    }

                    // Fallback to direct query
                    if let Ok(ctx_ash) = ctx.data::<Context<D>>() {
                        match rel_kind {
                            RelKind::BelongsTo => {
                                if let Some(foreign_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let query = CompiledQuery {
                                        filter: scoped_read_filter(
                                            dest_res,
                                            ctx_ash.actor.as_ref(),
                                            Some(Filter::eq(dest_attr, Value::Uuid(foreign_id))),
                                        ),
                                        tenant: ctx_ash.tenant.clone(),
                                        limit: Some(1),
                                        ..CompiledQuery::default()
                                    };
                                    if let Ok(mut records) =
                                        ctx_ash.data.run_query(dest_res, &query).await
                                        && let Some(mut rec) = records.pop()
                                    {
                                        let _ = redact_fields(
                                            dest_res,
                                            ctx_ash.actor.as_ref(),
                                            &mut rec,
                                        );
                                        return Ok(Some(FieldValue::owned_any(rec)));
                                    }
                                }
                                return Ok(None);
                            }
                            RelKind::HasOne => {
                                if let Some(source_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let query = CompiledQuery {
                                        filter: scoped_read_filter(
                                            dest_res,
                                            ctx_ash.actor.as_ref(),
                                            Some(Filter::eq(dest_attr, Value::Uuid(source_id))),
                                        ),
                                        tenant: ctx_ash.tenant.clone(),
                                        limit: Some(1),
                                        ..CompiledQuery::default()
                                    };
                                    if let Ok(mut records) =
                                        ctx_ash.data.run_query(dest_res, &query).await
                                        && let Some(mut rec) = records.pop()
                                    {
                                        let _ = redact_fields(
                                            dest_res,
                                            ctx_ash.actor.as_ref(),
                                            &mut rec,
                                        );
                                        return Ok(Some(FieldValue::owned_any(rec)));
                                    }
                                }
                                return Ok(None);
                            }
                            RelKind::HasMany => {
                                if let Some(source_id) =
                                    map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let query = CompiledQuery {
                                        filter: scoped_read_filter(
                                            dest_res,
                                            ctx_ash.actor.as_ref(),
                                            Some(Filter::eq(dest_attr, Value::Uuid(source_id))),
                                        ),
                                        tenant: ctx_ash.tenant.clone(),
                                        ..CompiledQuery::default()
                                    };
                                    if let Ok(records) =
                                        ctx_ash.data.run_query(dest_res, &query).await
                                    {
                                        let items: Vec<FieldValue> = records
                                            .into_iter()
                                            .map(|mut r| {
                                                let _ = redact_fields(
                                                    dest_res,
                                                    ctx_ash.actor.as_ref(),
                                                    &mut r,
                                                );
                                                FieldValue::owned_any(r)
                                            })
                                            .collect();
                                        return Ok(Some(FieldValue::list(items)));
                                    }
                                }
                                return Ok(Some(FieldValue::list(Vec::<FieldValue>::new())));
                            }
                            RelKind::ManyToMany => {
                                if let Some(through) = through_fn
                                    && let Some(src_join) = source_join
                                    && let Some(dst_join) = dest_join
                                    && let Some(source_id) =
                                        map.get(source_attr).and_then(|v| v.as_uuid())
                                {
                                    let join_query = CompiledQuery {
                                        filter: Some(Filter::eq(src_join, Value::Uuid(source_id))),
                                        tenant: ctx_ash.tenant.clone(),
                                        ..CompiledQuery::default()
                                    };
                                    if let Ok(join_rows) =
                                        ctx_ash.data.run_query(through(), &join_query).await
                                    {
                                        let dest_ids: Vec<Value> = join_rows
                                            .into_iter()
                                            .filter_map(|r| r.get(dst_join).cloned())
                                            .collect();
                                        if !dest_ids.is_empty() {
                                            let pk = dest_res
                                                .attributes
                                                .iter()
                                                .find(|a| a.primary_key)
                                                .map(|a| a.name)
                                                .unwrap_or("id");
                                            let dest_query = CompiledQuery {
                                                filter: scoped_read_filter(
                                                    dest_res,
                                                    ctx_ash.actor.as_ref(),
                                                    Some(Filter::in_list(pk, dest_ids)),
                                                ),
                                                tenant: ctx_ash.tenant.clone(),
                                                ..CompiledQuery::default()
                                            };
                                            if let Ok(records) =
                                                ctx_ash.data.run_query(dest_res, &dest_query).await
                                            {
                                                let items: Vec<FieldValue> = records
                                                    .into_iter()
                                                    .map(|mut r| {
                                                        let _ = redact_fields(
                                                            dest_res,
                                                            ctx_ash.actor.as_ref(),
                                                            &mut r,
                                                        );
                                                        FieldValue::owned_any(r)
                                                    })
                                                    .collect();
                                                return Ok(Some(FieldValue::list(items)));
                                            }
                                        }
                                    }
                                }
                                return Ok(Some(FieldValue::list(Vec::<FieldValue>::new())));
                            }
                        }
                    }
                }
                Ok(None)
            })
        });

        obj = obj.field(field);
    }

    obj
}

/// Collects all [`Enum`] types needed for atom attributes on a resource.
pub fn collect_enums_for_resource(resource: &'static ResourceDef) -> Vec<Enum> {
    let mut enums = Vec::new();
    for attr in resource.attributes {
        if let AttrType::Atom { one_of } = attr.ty {
            let enum_name = enum_type_name(resource.name, attr.name);
            let mut gql_enum = Enum::new(enum_name);
            for variant in one_of {
                let item_name = variant.to_uppercase();
                gql_enum = gql_enum.item(EnumItem::new(item_name));
            }
            enums.push(gql_enum);
        }
    }
    enums
}
