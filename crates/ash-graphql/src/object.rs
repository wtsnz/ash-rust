use ash_core::eval as eval_expr;
use ash_core::{AttrType, DataLayer, FieldMap, RelKind, RelatedQuery, RelationshipDef, ResourceDef, Value};

use async_graphql::Value as GqlValue;
use async_graphql::dataloader::DataLoader;
use async_graphql::dynamic::*;

use crate::dataloader::{AshBatchLoader, RelatedKey};
use crate::filter::resource_filter_input_name;
use crate::names::camel;
use crate::preload::{preloaded_key, related_query};
use crate::redact::{is_forbidden, redact_record, relationship_source, report_forbidden};
use crate::request::{request_actor, request_context};
use crate::sort::resource_sort_input_name;
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

        let field = Field::new(camel(attr_name), type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    let actor = request_actor::<D>(&ctx);

                    if has_field_policy && is_forbidden(resource, actor, map, attr_name) {
                        report_forbidden(&ctx);
                        return Ok(None);
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
        let has_field_policy = resource.field_policies.iter().any(|fp| fp.field == calc.name);

        let field = Field::new(camel(calc_name), type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    if has_field_policy && is_forbidden(resource, request_actor::<D>(&ctx), map, calc_name) {
                        report_forbidden(&ctx);
                        return Ok(None);
                    }
                    // Loaded with the record, nil included: computed from the record as
                    // stored, which the record here may not wholly hold.
                    match map.get(calc_name) {
                        Some(val) if val.is_null() => return Ok(None),
                        Some(val) => {
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value_typed(
                                val, calc_ty,
                            ))));
                        }
                        None => {}
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

    // 3. Aggregates. A count (or exists) always has a value; a sum or first of nothing
    // is null, as in AshGraphql.
    for agg in resource.aggregates {
        let agg_name = agg.name;
        // Unless a field policy may hide it, as one may an attribute.
        let has_field_policy = resource.field_policies.iter().any(|fp| fp.field == agg.name);
        let always = matches!(
            agg.kind,
            ash_core::AggregateKind::Count | ash_core::AggregateKind::Exists
        ) && !has_field_policy;
        let type_ref = attr_type_to_type_ref(resource.name, agg.name, agg.ty, !always);

        let field = Field::new(camel(agg_name), type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    if has_field_policy && is_forbidden(resource, request_actor::<D>(&ctx), map, agg_name) {
                        report_forbidden(&ctx);
                        return Ok(None);
                    }
                    match map.get(agg_name) {
                        Some(val) if !val.is_null() => {
                            return Ok(Some(FieldValue::value(ash_value_to_graphql_value(val))));
                        }
                        _ => {}
                    }
                }
                Ok(always.then(|| FieldValue::value(GqlValue::Number(0.into()))))
            })
        });

        obj = obj.field(field);
    }

    // 4. Relationships, each loaded as the request's context reads it. A belongs_to whose
    // key can't be nil is never null, unless policies may hide what it points to; a
    // to-many relationship takes the related resource's sort and filter, and a limit and
    // offset, as in AshGraphql.
    for rel in resource.relationships {
        let dest_res = (rel.destination)();
        let rel_name = rel.name;
        let to_one = matches!(rel.kind, RelKind::BelongsTo | RelKind::HasOne);
        // As AshGraphql's guide has it, a field a policy may hide is nullable, as if it
        // were in `nullable_fields`: a related record behind read policies may not load.
        let required = rel.kind == RelKind::BelongsTo
            && dest_res.policies.is_empty()
            && rel
                .source_columns()
                .iter()
                .all(|key| resource.attribute(key).is_some_and(|attr| !attr.allow_nil));
        let type_ref = if !to_one {
            TypeRef::named_nn_list_nn(dest_res.name)
        } else if required {
            TypeRef::named_nn(dest_res.name)
        } else {
            TypeRef::named(dest_res.name)
        };

        let mut field = Field::new(camel(rel_name), type_ref, move |ctx| {
            FieldFuture::new(async move {
                let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() else {
                    return Ok(None);
                };
                // Loaded ahead with its record, and already as the requester may see it.
                let field = ctx.ctx.field();
                if let Some(preloaded) = map.get(&preloaded_key(rel_name, &field)?) {
                    return Ok(match preloaded {
                        Value::Map(row) => Some(FieldValue::borrowed_any(row)),
                        Value::Array(rows) => Some(FieldValue::list(
                            rows.iter().filter_map(Value::as_map).map(|row| FieldValue::borrowed_any(row)),
                        )),
                        _ if to_one => None,
                        _ => Some(FieldValue::list(std::iter::empty::<FieldValue>())),
                    });
                }
                let query = if to_one {
                    RelatedQuery::default()
                } else {
                    related_query(dest_res, request_actor::<D>(&ctx), ctx.args.iter().map(|(name, value)| (name.as_str(), value.as_value())))?
                };
                let shaped = query.filter.is_some()
                    || !query.sort.is_empty()
                    || query.limit.is_some()
                    || query.offset.is_some();
                let rows = match map.get(rel_name) {
                    _ if shaped => load_shaped::<D>(&ctx, resource, rel, map, &query).await?,
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
                    redact_record(dest_res, actor, &mut row);
                    FieldValue::owned_any(row)
                });
                if to_one {
                    Ok(items.next())
                } else {
                    Ok(Some(FieldValue::list(items)))
                }
            })
        });

        if !to_one {
            field = field
                .argument(InputValue::new(
                    "sort",
                    TypeRef::named_list(resource_sort_input_name(dest_res.name)),
                ))
                .argument(InputValue::new(
                    "filter",
                    TypeRef::named(resource_filter_input_name(dest_res.name)),
                ))
                .argument(InputValue::new("limit", TypeRef::named(TypeRef::INT)))
                .argument(InputValue::new("offset", TypeRef::named(TypeRef::INT)));
        }
        obj = obj.field(field);
    }

    obj
}

/// `source`'s related rows for a to-many relationship read with a sort, filter, limit or
/// offset, where they weren't loaded ahead with it: its related query on its own.
async fn load_shaped<D: DataLayer + Clone + 'static>(
    ctx: &ResolverContext<'_>,
    resource: &'static ResourceDef,
    rel: &'static RelationshipDef,
    source: &FieldMap,
    query: &RelatedQuery,
) -> async_graphql::Result<Vec<FieldMap>> {
    let ash = request_context::<D>(ctx)?;
    let mut related =
        ash_core::load_related_query(&*ash, resource, rel.name, &[relationship_source(rel, source)], query)
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    Ok(related.pop().unwrap_or_default())
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
        ash_core::load_related(&*ash, resource, rel.name, &[relationship_source(rel, source)])
            .await
            .map_err(|e| async_graphql::Error::new(e.to_string()))?;
    Ok(related.pop().unwrap_or_default())
}

/// The enum types a resource's attributes, aggregates and calculations use: one per
/// named atom, with its values upper-cased.
pub fn collect_enums_for_resource(resource: &'static ResourceDef) -> Vec<Enum> {
    let types = resource
        .attributes
        .iter()
        .map(|attr| attr.ty)
        .chain(resource.aggregates.iter().map(|agg| agg.ty))
        .chain(resource.calculations.iter().map(|calc| calc.ty))
        .chain(resource.actions.iter().flat_map(|action| action.arguments.iter().map(|arg| arg.ty)));
    let mut enums: Vec<Enum> = Vec::new();
    let mut seen = Vec::new();
    for ty in types {
        if let (Some(name), AttrType::Atom { one_of, .. }) = (enum_type_name(ty), ty)
            && !seen.contains(&name)
        {
            seen.push(name);
            let mut gql_enum = Enum::new(name);
            for variant in one_of {
                gql_enum = gql_enum.item(EnumItem::new(variant.to_uppercase()));
            }
            enums.push(gql_enum);
        }
    }
    enums
}
