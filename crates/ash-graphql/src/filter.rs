use ash_core::{AttrType, Filter, ResourceDef, Value};
use async_graphql::dynamic::*;

use crate::types::enum_type_name;

/// Registers common primitive filter input objects.
pub fn register_primitive_filter_inputs(mut builder: SchemaBuilder) -> SchemaBuilder {
    // StringFilterInput
    let str_filter = InputObject::new("StringFilterInput")
        .field(InputValue::new("eq", TypeRef::named(TypeRef::STRING)))
        .field(InputValue::new("ne", TypeRef::named(TypeRef::STRING)))
        .field(InputValue::new(
            "in",
            TypeRef::named_list(TypeRef::STRING),
        ))
        .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)));
    builder = builder.register(str_filter);

    // IntFilterInput
    let int_filter = InputObject::new("IntFilterInput")
        .field(InputValue::new("eq", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("ne", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("gt", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("gte", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("lt", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("lte", TypeRef::named(TypeRef::INT)))
        .field(InputValue::new("in", TypeRef::named_list(TypeRef::INT)))
        .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)));
    builder = builder.register(int_filter);

    // BooleanFilterInput
    let bool_filter = InputObject::new("BooleanFilterInput")
        .field(InputValue::new("eq", TypeRef::named(TypeRef::BOOLEAN)))
        .field(InputValue::new("ne", TypeRef::named(TypeRef::BOOLEAN)))
        .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)));
    builder = builder.register(bool_filter);

    // UuidFilterInput
    let uuid_filter = InputObject::new("UuidFilterInput")
        .field(InputValue::new("eq", TypeRef::named(TypeRef::ID)))
        .field(InputValue::new("ne", TypeRef::named(TypeRef::ID)))
        .field(InputValue::new("in", TypeRef::named_list(TypeRef::ID)))
        .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)));
    builder = builder.register(uuid_filter);

    builder
}

/// Generates the `<Resource>FilterInput` name.
pub fn resource_filter_input_name(resource_name: &str) -> String {
    format!("{}FilterInput", resource_name)
}

/// Generates an enum filter input name.
pub fn enum_filter_input_name(resource_name: &str, field_name: &str) -> String {
    format!("{}{}EnumFilterInput", resource_name, field_name)
}

/// Registers resource-specific filter inputs for a [`ResourceDef`].
pub fn register_resource_filter_inputs(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let filter_name = resource_filter_input_name(resource.name);
    let mut res_filter = InputObject::new(filter_name.clone());

    for attr in resource.attributes {
        let field_filter_type = match attr.ty {
            AttrType::Uuid => "UuidFilterInput".to_string(),
            AttrType::String => "StringFilterInput".to_string(),
            AttrType::Integer => "IntFilterInput".to_string(),
            AttrType::Boolean => "BooleanFilterInput".to_string(),
            AttrType::Atom { .. } => {
                let e_filter_name = enum_filter_input_name(resource.name, attr.name);
                let e_name = enum_type_name(resource.name, attr.name);
                let e_filter = InputObject::new(e_filter_name.clone())
                    .field(InputValue::new("eq", TypeRef::named(&e_name)))
                    .field(InputValue::new("ne", TypeRef::named(&e_name)))
                    .field(InputValue::new("in", TypeRef::named_list(&e_name)))
                    .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)));
                builder = builder.register(e_filter);
                e_filter_name
            }
            _ => continue,
        };

        res_filter = res_filter.field(InputValue::new(attr.name, TypeRef::named(field_filter_type)));
    }

    // Relationships (nested filters)
    for rel in resource.relationships {
        let dest = (rel.destination)();
        let dest_filter_name = resource_filter_input_name(dest.name);
        res_filter = res_filter.field(InputValue::new(rel.name, TypeRef::named(dest_filter_name)));
    }

    // Boolean combinators
    res_filter = res_filter
        .field(InputValue::new(
            "and",
            TypeRef::named_list(&filter_name),
        ))
        .field(InputValue::new(
            "or",
            TypeRef::named_list(&filter_name),
        ))
        .field(InputValue::new("not", TypeRef::named(&filter_name)));

    builder.register(res_filter)
}

/// Parses a dynamic [`ObjectAccessor`] representing a `<Resource>FilterInput` into an Ash [`Filter`].
pub fn parse_resource_filter(
    resource: &'static ResourceDef,
    obj: &ObjectAccessor<'_>,
) -> Result<Filter, async_graphql::Error> {
    let mut filters = Vec::new();

    // 1. Attribute filters
    for attr in resource.attributes {
        if let Some(attr_filter_val) = obj.get(attr.name) {
            let attr_obj = attr_filter_val.object()?;
            let field_name = attr.name;

            // eq
            if let Some(eq_val) = attr_obj.get("eq") {
                let val = parse_scalar_value(&eq_val, attr.ty)?;
                filters.push(Filter::eq(field_name, val));
            }

            // ne
            if let Some(ne_val) = attr_obj.get("ne") {
                let val = parse_scalar_value(&ne_val, attr.ty)?;
                filters.push(Filter::ne(field_name, val));
            }

            // gt
            if let Some(gt_val) = attr_obj.get("gt") {
                let val = parse_scalar_value(&gt_val, attr.ty)?;
                filters.push(Filter::gt(field_name, val));
            }

            // gte
            if let Some(gte_val) = attr_obj.get("gte") {
                let val = parse_scalar_value(&gte_val, attr.ty)?;
                filters.push(Filter::gte(field_name, val));
            }

            // lt
            if let Some(lt_val) = attr_obj.get("lt") {
                let val = parse_scalar_value(&lt_val, attr.ty)?;
                filters.push(Filter::lt(field_name, val));
            }

            // lte
            if let Some(lte_val) = attr_obj.get("lte") {
                let val = parse_scalar_value(&lte_val, attr.ty)?;
                filters.push(Filter::lte(field_name, val));
            }

            // in
            if let Some(in_val) = attr_obj.get("in") {
                let list = in_val.list()?;
                let mut vals = Vec::new();
                for item in list.iter() {
                    vals.push(parse_scalar_value(&item, attr.ty)?);
                }
                filters.push(Filter::in_list(field_name, vals));
            }

            // isNil
            if let Some(is_nil_val) = attr_obj.get("isNil") {
                if is_nil_val.boolean()? {
                    filters.push(Filter::is_nil(field_name));
                } else {
                    filters.push(Filter::Not(Box::new(Filter::is_nil(field_name))));
                }
            }
        }
    }

    // 2. Relationship filters
    for rel in resource.relationships {
        if let Some(rel_filter_val) = obj.get(rel.name) {
            let rel_obj = rel_filter_val.object()?;
            let dest = (rel.destination)();
            let dest_filter = parse_resource_filter(dest, &rel_obj)?;
            if dest_filter != Filter::True {
                filters.push(Filter::related(rel.name, dest_filter));
            }
        }
    }

    // 3. Boolean combinator 'and'
    if let Some(and_val) = obj.get("and") {
        let list = and_val.list()?;
        let mut sub_filters = Vec::new();
        for item in list.iter() {
            let sub_obj = item.object()?;
            sub_filters.push(parse_resource_filter(resource, &sub_obj)?);
        }
        if !sub_filters.is_empty() {
            filters.push(Filter::And(sub_filters));
        }
    }

    // 3. Boolean combinator 'or'
    if let Some(or_val) = obj.get("or") {
        let list = or_val.list()?;
        let mut sub_filters = Vec::new();
        for item in list.iter() {
            let sub_obj = item.object()?;
            sub_filters.push(parse_resource_filter(resource, &sub_obj)?);
        }
        if !sub_filters.is_empty() {
            filters.push(Filter::Or(sub_filters));
        }
    }

    // 4. Boolean combinator 'not'
    if let Some(not_val) = obj.get("not") {
        let sub_obj = not_val.object()?;
        let inner = parse_resource_filter(resource, &sub_obj)?;
        filters.push(Filter::Not(Box::new(inner)));
    }

    if filters.is_empty() {
        Ok(Filter::True)
    } else if filters.len() == 1 {
        Ok(filters.remove(0))
    } else {
        Ok(Filter::And(filters))
    }
}

fn parse_scalar_value(
    acc: &ValueAccessor<'_>,
    ty: AttrType,
) -> Result<Value, async_graphql::Error> {
    match ty {
        AttrType::Uuid => {
            let s = acc.string()?;
            let u = uuid::Uuid::parse_str(s)
                .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;
            Ok(Value::Uuid(u))
        }
        AttrType::String => {
            let s = acc.string()?;
            Ok(Value::String(s.to_string()))
        }
        AttrType::Integer => {
            let n = acc.i64()?;
            Ok(Value::Int(n))
        }
        AttrType::Boolean => {
            let b = acc.boolean()?;
            Ok(Value::Bool(b))
        }
        AttrType::Atom { one_of } => {
            let name = acc.enum_name()?;
            if let Some(matched) = one_of.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
                Ok(Value::String((*matched).to_string()))
            } else {
                Ok(Value::String(name.to_string()))
            }
        }
        _ => Ok(Value::Null),
    }
}
