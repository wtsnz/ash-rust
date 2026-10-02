//! Sort inputs, as AshGraphql generates them: `<Resource>SortInput { order: SortOrder,
//! field: <Resource>SortField! }`, sortable by any attribute, aggregate or calculation.

use ash_core::{ResourceDef, Sort};
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

/// The shared `SortOrder` enum. ash-core doesn't place nulls, so the `*_NULLS_*` orders
/// sort as their direction does.
pub fn register_sort_order(builder: SchemaBuilder) -> SchemaBuilder {
    let mut order = Enum::new("SortOrder");
    for item in ["DESC", "DESC_NULLS_FIRST", "DESC_NULLS_LAST", "ASC", "ASC_NULLS_FIRST", "ASC_NULLS_LAST"] {
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

/// Parses a `[<Resource>SortInput]` into sorts.
pub fn parse_resource_sort(
    resource: &'static ResourceDef,
    list: &ListAccessor<'_>,
) -> Result<Vec<Sort>, async_graphql::Error> {
    let mut sorts = Vec::new();
    for item in list.iter() {
        let obj = item.object()?;
        let wanted = obj
            .get("field")
            .ok_or_else(|| async_graphql::Error::new("Missing required sort field"))?;
        let wanted = wanted.enum_name()?;
        let field = sort_fields(resource)
            .find(|field| upper_snake(field) == wanted)
            .ok_or_else(|| async_graphql::Error::new(format!("Unknown sort field `{wanted}`")))?;
        let descending = match obj.get("order") {
            Some(order) if !order.is_null() => order.enum_name()?.starts_with("DESC"),
            _ => false,
        };
        sorts.push(Sort {
            field: field.to_string(),
            descending,
        });
    }
    Ok(sorts)
}
