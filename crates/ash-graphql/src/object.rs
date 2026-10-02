use ash_core::eval as eval_expr;
use ash_core::redact_fields;
use ash_core::{AttrType, DataLayer, FieldMap, RelKind, RelationshipDef, ResourceDef, Value};

use async_graphql::Value as GqlValue;
use async_graphql::dataloader::DataLoader;
use async_graphql::dynamic::*;

use crate::dataloader::{AshBatchLoader, RelatedKey};
use crate::request::{request_actor, request_context};
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
                    let actor = request_actor::<D>(&ctx);

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

    // 4. Relationships, each loaded as the request's context reads it.
    for rel in resource.relationships {
        let dest_res = (rel.destination)();
        let rel_name = rel.name;
        let to_one = matches!(rel.kind, RelKind::BelongsTo | RelKind::HasOne);
        let type_ref = if to_one {
            TypeRef::named(dest_res.name)
        } else {
            TypeRef::named_nn_list_nn(dest_res.name)
        };

        let field = Field::new(rel_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() else {
                    return Ok(None);
                };
                let rows = match map.get(rel_name) {
                    // Loaded along with the parent.
                    Some(Value::Null) => Vec::new(),
                    Some(Value::Map(m)) => vec![m.clone()],
                    Some(Value::Array(items)) => {
                        items.iter().filter_map(|v| v.as_map().cloned()).collect()
                    }
                    _ => load_relationship::<D>(&ctx, resource, rel, map).await?,
                };
                let actor = request_actor::<D>(&ctx);
                let mut items = rows.into_iter().map(|mut row| {
                    let _ = redact_fields(dest_res, actor, &mut row);
                    FieldValue::owned_any(row)
                });
                if to_one {
                    Ok(items.next())
                } else {
                    Ok(Some(FieldValue::list(items)))
                }
            })
        });

        obj = obj.field(field);
    }

    obj
}

/// `source`'s `rel` rows, batched through the request's dataloader when it reads as the
/// request does.
async fn load_relationship<D: DataLayer + Clone + 'static>(
    ctx: &ResolverContext<'_>,
    resource: &'static ResourceDef,
    rel: &'static RelationshipDef,
    source: &FieldMap,
) -> async_graphql::Result<Vec<FieldMap>> {
    let ash = request_context::<D>(ctx)?;
    if let Some(loader) = ctx.data_opt::<DataLoader<AshBatchLoader<D>>>()
        && loader.loader().serves(&ash)
    {
        let rows = loader
            .load_one(RelatedKey::new(resource, rel, source))
            .await
            .map_err(|e| (*e).clone())?;
        return Ok(rows.unwrap_or_default());
    }
    let mut related =
        ash_core::load_related(&*ash, resource, rel.name, std::slice::from_ref(source))
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    Ok(related.pop().unwrap_or_default())
}

/// Collects all [`Enum`] types needed for atom attributes on a resource.
pub fn collect_enums_for_resource(resource: &'static ResourceDef) -> Vec<Enum> {
    let mut enums = Vec::new();
    for attr in resource.attributes {
        if let AttrType::Atom { one_of, .. } = attr.ty {
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
