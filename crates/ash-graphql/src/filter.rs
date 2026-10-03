//! Filter inputs, as AshGraphql generates them: `<Resource>FilterInput` with `and`, `or`
//! and `not` lists, a `<Resource>Filter<Field>` input for each attribute, aggregate and
//! calculation, and the related resource's filter input for each relationship.

use ash_core::{AttrType, Filter, ResourceDef, Value};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::names::{camel, pascal};
use crate::types::{graphql_type_name, parse_input_value};

/// `Trip` → `TripFilterInput`.
pub fn resource_filter_input_name(resource_name: &str) -> String {
    format!("{resource_name}FilterInput")
}

/// `Trip`, `requested_at` → `TripFilterRequestedAt`.
pub fn field_filter_input_name(resource_name: &str, field: &str) -> String {
    format!("{resource_name}Filter{}", pascal(field))
}

/// Text types also take substring and pattern operators.
fn is_text(ty: AttrType) -> bool {
    matches!(ty, AttrType::String | AttrType::CiString)
}

/// Whether a field of this type can be filtered at all.
fn filterable(ty: AttrType) -> bool {
    !matches!(ty, AttrType::Map | AttrType::Array | AttrType::Vector { .. } | AttrType::Binary)
}

/// The operators every filterable field takes, and those text fields add.
const OPERATORS: &[&str] = &[
    "eq",
    "notEq",
    "lessThan",
    "greaterThan",
    "lessThanOrEqual",
    "greaterThanOrEqual",
    "isDistinctFrom",
    "isNotDistinctFrom",
];
const TEXT_OPERATORS: &[&str] = &["contains", "stringStartsWith", "stringEndsWith", "like", "ilike"];

fn field_filter(name: String, ty: AttrType, items_nullable: bool) -> InputObject {
    let scalar = graphql_type_name(ty);
    let mut input = InputObject::new(name)
        .field(InputValue::new("isNil", TypeRef::named(TypeRef::BOOLEAN)))
        .field(InputValue::new(
            "in",
            if items_nullable {
                TypeRef::named_list(scalar)
            } else {
                TypeRef::named_nn_list(scalar)
            },
        ));
    for op in OPERATORS {
        input = input.field(InputValue::new(*op, TypeRef::named(scalar)));
    }
    if is_text(ty) {
        for op in TEXT_OPERATORS {
            input = input.field(InputValue::new(*op, TypeRef::named(TypeRef::STRING)));
        }
    }
    input
}

/// Registers `<Resource>FilterInput` and its field inputs.
pub fn register_resource_filter_inputs(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let name = resource_filter_input_name(resource.name);
    let mut filter = InputObject::new(&name)
        .field(InputValue::new("and", TypeRef::named_nn_list(&name)))
        .field(InputValue::new("or", TypeRef::named_nn_list(&name)))
        .field(InputValue::new("not", TypeRef::named_nn_list(&name)));

    let mut add = |builder: SchemaBuilder, field: &str, ty: AttrType, items_nullable: bool| {
        if !filterable(ty) {
            return builder;
        }
        let input = field_filter_input_name(resource.name, field);
        filter = std::mem::replace(&mut filter, InputObject::new("_"))
            .field(InputValue::new(camel(field), TypeRef::named(&input)));
        builder.register(field_filter(input, ty, items_nullable))
    };
    for attr in resource.attributes {
        builder = add(builder, attr.name, attr.ty, attr.allow_nil);
    }
    for agg in resource.aggregates {
        builder = add(builder, agg.name, agg.ty, true);
    }
    for calc in resource.calculations {
        builder = add(builder, calc.name, calc.ty, false);
    }
    for rel in resource.relationships {
        let destination = (rel.destination)();
        filter = filter.field(InputValue::new(
            camel(rel.name),
            TypeRef::named(resource_filter_input_name(destination.name)),
        ));
    }
    builder.register(filter)
}

/// The type of `field` on `resource`: an attribute, aggregate or calculation.
fn field_type(resource: &ResourceDef, field: &str) -> Option<AttrType> {
    resource
        .attribute(field)
        .map(|attr| attr.ty)
        .or_else(|| resource.aggregate(field).map(|agg| agg.ty))
        .or_else(|| resource.calculation(field).map(|calc| calc.ty))
}

/// Parses a `<Resource>FilterInput` into a [`Filter`] as `actor` may run it: fields its
/// field policies hide read as null where they're hidden, as Ash reads a client's filter
/// ([`ash_core::guard_input_filter`]). It reads the value as given (an argument's, or one
/// a selection holds), so a relationship selected with a filter can be loaded ahead of
/// its field.
pub fn parse_resource_filter(
    resource: &'static ResourceDef,
    actor: Option<&ash_core::Actor>,
    value: &GqlValue,
) -> Result<Filter, async_graphql::Error> {
    let filter = parse_filter(resource, value)?;
    ash_core::guard_input_filter(resource, actor, filter).map_err(|e| async_graphql::Error::new(e.to_string()))
}

fn parse_filter(resource: &'static ResourceDef, value: &GqlValue) -> Result<Filter, async_graphql::Error> {
    let obj = object(value)?;
    let mut filters = Vec::new();

    let fields = resource
        .attributes
        .iter()
        .map(|attr| attr.name)
        .chain(resource.aggregates.iter().map(|agg| agg.name))
        .chain(resource.calculations.iter().map(|calc| calc.name));
    for field in fields {
        let Some(ops) = obj.get(camel(field).as_str()) else {
            continue;
        };
        if matches!(ops, GqlValue::Null) {
            continue;
        }
        let ty = field_type(resource, field).expect("a field of the resource");
        filters.extend(parse_field_filter(field, ty, object(ops)?)?);
    }

    for rel in resource.relationships {
        if let Some(nested) = obj.get(camel(rel.name).as_str())
            && !matches!(nested, GqlValue::Null)
        {
            let destination = (rel.destination)();
            let inner = parse_filter(destination, nested)?;
            if inner != Filter::True {
                filters.push(Filter::related(rel.name, inner));
            }
        }
    }

    let list = |key: &str| -> Result<Vec<Filter>, async_graphql::Error> {
        let mut parts = Vec::new();
        if let Some(value) = obj.get(key)
            && !matches!(value, GqlValue::Null)
        {
            for item in list(value) {
                parts.push(parse_filter(resource, item)?);
            }
        }
        Ok(parts)
    };
    let and = list("and")?;
    if !and.is_empty() {
        filters.push(Filter::And(and));
    }
    let or = list("or")?;
    if !or.is_empty() {
        filters.push(Filter::Or(or));
    }
    // `not: [a, b]` excludes records matching all of them, as AshGraphql reads it.
    let not = list("not")?;
    if !not.is_empty() {
        filters.push(Filter::Not(Box::new(Filter::And(not))));
    }

    Ok(match filters.len() {
        0 => Filter::True,
        1 => filters.remove(0),
        _ => Filter::And(filters),
    })
}

/// The input object `value` holds.
fn object(value: &GqlValue) -> Result<&async_graphql::indexmap::IndexMap<async_graphql::Name, GqlValue>, async_graphql::Error> {
    match value {
        GqlValue::Object(obj) => Ok(obj),
        _ => Err(async_graphql::Error::new("internal: not an object")),
    }
}

/// The items of a list input, or the one value given in its place, as GraphQL coerces it.
pub(crate) fn list(value: &GqlValue) -> &[GqlValue] {
    match value {
        GqlValue::List(items) => items,
        single => std::slice::from_ref(single),
    }
}

fn parse_field_filter(
    field: &str,
    ty: AttrType,
    ops: &async_graphql::indexmap::IndexMap<async_graphql::Name, GqlValue>,
) -> Result<Vec<Filter>, async_graphql::Error> {
    let mut filters = Vec::new();
    for (op, value) in ops {
        let op = op.as_str();
        if matches!(value, GqlValue::Null) && !matches!(op, "eq" | "isDistinctFrom" | "isNotDistinctFrom") {
            continue;
        }
        let parse = |value: &GqlValue| -> Result<Value, async_graphql::Error> {
            if matches!(value, GqlValue::Null) {
                Ok(Value::Null)
            } else {
                parse_input_value(value, ty)
            }
        };
        let text = || -> Result<String, async_graphql::Error> {
            match value {
                GqlValue::String(text) => Ok(text.clone()),
                _ => Err(async_graphql::Error::new("internal: not a string")),
            }
        };
        filters.push(match op {
            "isNil" => {
                let GqlValue::Boolean(is_nil) = value else {
                    return Err(async_graphql::Error::new("internal: not a boolean"));
                };
                if *is_nil {
                    Filter::is_nil(field)
                } else {
                    !Filter::is_nil(field)
                }
            }
            "eq" => Filter::eq(field, parse(value)?),
            "notEq" => Filter::ne(field, parse(value)?),
            "lessThan" => Filter::lt(field, parse(value)?),
            "greaterThan" => Filter::gt(field, parse(value)?),
            "lessThanOrEqual" => Filter::lte(field, parse(value)?),
            "greaterThanOrEqual" => Filter::gte(field, parse(value)?),
            "in" => {
                let mut values = Vec::new();
                for item in list(value) {
                    values.push(parse(item)?);
                }
                Filter::in_list(field, values)
            }
            // Null-safe equality: nulls are equal to each other and to nothing else.
            "isDistinctFrom" => match parse(value)? {
                Value::Null => !Filter::is_nil(field),
                v => Filter::or([Filter::is_nil(field), Filter::ne(field, v)]),
            },
            "isNotDistinctFrom" => match parse(value)? {
                Value::Null => Filter::is_nil(field),
                v => Filter::eq(field, v),
            },
            "contains" => Filter::contains(field, text()?),
            "stringStartsWith" => Filter::starts_with(field, text()?),
            "stringEndsWith" => Filter::ends_with(field, text()?),
            "like" => Filter::like(field, text()?),
            "ilike" => Filter::ilike(field, text()?),
            other => {
                return Err(async_graphql::Error::new(format!(
                    "Unknown filter operator `{other}` on `{}`",
                    camel(field)
                )));
            }
        });
    }
    Ok(filters)
}
