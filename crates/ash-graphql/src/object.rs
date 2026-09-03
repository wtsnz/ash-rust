use ash_core::eval as eval_expr;
use ash_core::redact_fields;
use ash_core::{Actor, AttrType, FieldMap, ResourceDef};
use async_graphql::dynamic::*;
use async_graphql::Value as GqlValue;

use crate::types::{
    ash_value_to_graphql_value, ash_value_to_graphql_value_typed, attr_type_to_type_ref,
    enum_type_name,
};

/// Builds the GraphQL [`Object`] definition for an Ash [`ResourceDef`].
pub fn build_resource_object(resource: &'static ResourceDef) -> Object {
    let mut obj = Object::new(resource.name);

    // 1. Attributes
    for attr in resource.attributes {
        let attr_name = attr.name;
        let attr_ty = attr.ty;
        // If field policy protects this attribute, make it nullable so redaction does not null-fail the parent object
        let has_field_policy = resource.field_policies.iter().any(|fp| fp.field == attr.name);
        let allow_nil = attr.allow_nil || has_field_policy;
        let type_ref = attr_type_to_type_ref(resource.name, attr.name, attr.ty, allow_nil);

        let field = Field::new(attr_name, type_ref, move |ctx| {
            FieldFuture::new(async move {
                if let Some(map) = ctx.parent_value.downcast_ref::<FieldMap>() {
                    // Extract actor if present in context data
                    let actor = ctx.data_opt::<Actor>().or_else(|| {
                        ctx.data_opt::<Option<Actor>>().and_then(|opt| opt.as_ref())
                    });

                    // If field policy is present, check read authorization
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
                // Default to 0 for count, null for others
                Ok(Some(FieldValue::value(GqlValue::Number(0.into()))))
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
                // GraphQL enum items must be valid identifiers, typically uppercase
                let item_name = variant.to_uppercase();
                gql_enum = gql_enum.item(EnumItem::new(item_name));
            }
            enums.push(gql_enum);
        }
    }
    enums
}
