//! Sort inputs, as AshGraphql generates them: `<Resource>SortInput { order: SortOrder,
//! field: <Resource>SortField! }`, sortable by any attribute, aggregate or calculation.

use ash_core::{ResourceDef, Sort};
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::names::upper_snake;

/// `Trip` → `TripSortField`.
pub fn resource_sort_field_enum_name(resource_name: &str) -> String {
    format!("{resource_name}SortField")
}

/// `Trip` → `TripSortInput`.
pub fn resource_sort_input_name(resource_name: &str) -> String {
    format!("{resource_name}SortInput")
}

/// The shared `SortOrder` enum: a direction, and where nulls go (by default last
/// ascending and first descending).
pub fn register_sort_order(builder: SchemaBuilder) -> SchemaBuilder {
    let mut order = Enum::new("SortOrder");
    for item in [
        "DESC",
        "DESC_NULLS_FIRST",
        "DESC_NULLS_LAST",
        "ASC",
        "ASC_NULLS_FIRST",
        "ASC_NULLS_LAST",
    ] {
        order = order.item(EnumItem::new(item));
    }
    builder.register(order)
}

/// The fields a resource sorts by: its attributes, aggregates, and calculations that
/// take no arguments.
fn sort_fields(resource: &ResourceDef) -> impl Iterator<Item = &'static str> + '_ {
    resource
        .attributes
        .iter()
        .map(|attr| attr.name)
        .chain(resource.aggregates.iter().map(|agg| agg.name))
        .chain(
            resource
                .calculations
                .iter()
                .filter(|calc| calc.arguments.is_empty())
                .map(|calc| calc.name),
        )
}

/// Registers `<Resource>SortField` and `<Resource>SortInput`.
pub fn register_resource_sort_inputs(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    let enum_name = resource_sort_field_enum_name(resource.name);
    let mut fields = Enum::new(&enum_name);
    for field in sort_fields(resource) {
        fields = fields.item(EnumItem::new(upper_snake(field)));
    }
    builder = builder.register(fields);
    builder.register(
        InputObject::new(resource_sort_input_name(resource.name))
            .field(InputValue::new("order", TypeRef::named("SortOrder")))
            .field(InputValue::new("field", TypeRef::named_nn(enum_name))),
    )
}

/// Parse client sort input. Core applies the bound reader's field visibility through
/// [`ash_core::plan_client_sort`] before the read runs.
pub fn parse_resource_sort(
    resource: &'static ResourceDef,
    value: &GqlValue,
) -> Result<ash_core::ClientExpr<Vec<Sort>>, async_graphql::Error> {
    let mut sorts = Vec::new();
    for item in crate::filter::list(value) {
        let GqlValue::Object(obj) = item else {
            return Err(async_graphql::Error::new("internal: not an object"));
        };
        let wanted = obj
            .get("field")
            .ok_or_else(|| async_graphql::Error::new("Missing required sort field"))?;
        let wanted = enum_name(wanted)?;
        let field = sort_fields(resource)
            .find(|field| upper_snake(field) == wanted)
            .ok_or_else(|| async_graphql::Error::new(format!("Unknown sort field `{wanted}`")))?;
        let order = match obj.get("order") {
            Some(order) if !matches!(order, GqlValue::Null) => enum_name(order)?,
            _ => "ASC",
        };
        let descending = order.starts_with("DESC");
        let nulls_first = if order.ends_with("_NULLS_FIRST") {
            Some(true)
        } else if order.ends_with("_NULLS_LAST") {
            Some(false)
        } else {
            None
        };
        sorts.push(Sort {
            field: field.to_string(),
            descending,
            nulls_first,
            ..Default::default()
        });
    }
    Ok(ash_core::ClientExpr::new(sorts))
}

/// An enum value's name: a literal's, or a variable's string.
fn enum_name(value: &GqlValue) -> Result<&str, async_graphql::Error> {
    match value {
        GqlValue::Enum(name) => Ok(name.as_str()),
        GqlValue::String(name) => Ok(name.as_str()),
        _ => Err(async_graphql::Error::new("internal: not an enum name")),
    }
}
