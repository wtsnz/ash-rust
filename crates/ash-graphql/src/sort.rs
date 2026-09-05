use ash_core::{ResourceDef, Sort};
use async_graphql::dynamic::*;

/// Generates the `<Resource>SortFieldEnum` name.
pub fn resource_sort_field_enum_name(resource_name: &str) -> String {
    format!("{}SortFieldEnum", resource_name)
}

/// Generates the `<Resource>SortInput` name.
pub fn resource_sort_input_name(resource_name: &str) -> String {
    format!("{}SortInput", resource_name)
}

/// Registers the global `SortOrderEnum` and resource-specific sort inputs.
pub fn register_resource_sort_inputs(
    mut builder: SchemaBuilder,
    resource: &'static ResourceDef,
) -> SchemaBuilder {
    // 1. SortOrderEnum
    let order_enum = Enum::new("SortOrderEnum")
        .item(EnumItem::new("ASC"))
        .item(EnumItem::new("DESC"))
        .item(EnumItem::new("asc"))
        .item(EnumItem::new("desc"));
    builder = builder.register(order_enum);

    // 2. <Resource>SortFieldEnum
    let field_enum_name = resource_sort_field_enum_name(resource.name);
    let mut field_enum = Enum::new(field_enum_name.clone());
    for attr in resource.attributes {
        field_enum = field_enum.item(EnumItem::new(attr.name.to_uppercase()));
        if attr.name != attr.name.to_uppercase() {
            field_enum = field_enum.item(EnumItem::new(attr.name));
        }
    }
    builder = builder.register(field_enum);

    // 3. <Resource>SortInput
    let sort_input_name = resource_sort_input_name(resource.name);
    let sort_input = InputObject::new(sort_input_name)
        .field(InputValue::new("field", TypeRef::named_nn(field_enum_name)))
        .field(InputValue::new("order", TypeRef::named("SortOrderEnum")));
    builder = builder.register(sort_input);

    builder
}

/// Parses a dynamic [`ListAccessor`] of `<Resource>SortInput` into a `Vec<ash_core::Sort>`.
pub fn parse_resource_sort(
    resource: &'static ResourceDef,
    list: &ListAccessor<'_>,
) -> Result<Vec<Sort>, async_graphql::Error> {
    let mut sorts = Vec::new();

    for item in list.iter() {
        let obj = item.object()?;
        let field_enum_val = obj
            .get("field")
            .ok_or_else(|| async_graphql::Error::new("Missing required sort field"))?;
        let field_name_upper = field_enum_val.enum_name()?;

        // Match against attributes
        let attr = resource
            .attributes
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case(field_name_upper))
            .ok_or_else(|| {
                async_graphql::Error::new(format!("Unknown sort field `{field_name_upper}`"))
            })?;

        let descending = if let Some(order_val) = obj.get("order") {
            let order = order_val.enum_name()?;
            order.eq_ignore_ascii_case("DESC")
        } else {
            false
        };

        sorts.push(Sort {
            field: attr.name.to_string(),
            descending,
        });
    }

    Ok(sorts)
}
