//! Derived-value transport shared with data layers. Only the presenter exposes values.

use crate::{Actor, FieldMap, ResourceDef, Result, Sort, Value};

const PREFIX: &str = "__ash_projected_";

/// Private query-result column carrying a projected derived value alongside its
/// trusted value. The presenter ignores undeclared columns.
pub fn projected_name(field: &str) -> String {
    format!("{PREFIX}{field}")
}

/// SQL aliases use the aggregate's ordinal to stay below database identifier limits.
pub fn projected_column(resource: &ResourceDef, field: &str) -> String {
    let index = resource
        .aggregates
        .iter()
        .position(|aggregate| aggregate.name == field)
        .expect("projected aggregate belongs to its resource");
    format!("{PREFIX}{index}")
}

pub fn projected_value<'a>(fields: &'a FieldMap, field: &str) -> Option<&'a Value> {
    fields
        .get(&projected_name(field))
        .or_else(|| fields.get(field))
}

pub fn sort_value<'a>(fields: &'a FieldMap, sort: &Sort) -> Option<&'a Value> {
    if sort.projected {
        projected_value(fields, &sort.field)
    } else {
        fields.get(&sort.field)
    }
}

pub fn projected_fields(fields: &FieldMap) -> FieldMap {
    into_projected_fields(fields.clone())
}

pub fn into_projected_fields(mut fields: FieldMap) -> FieldMap {
    let values: Vec<_> = fields
        .iter()
        .filter_map(|(name, value)| {
            name.strip_prefix(PREFIX)
                .map(|field| (field.to_string(), value.clone()))
        })
        .collect();
    for (name, value) in values {
        fields.remove(&projected_name(&name));
        if let Some(field) = fields.get_mut(&name) {
            *field = value;
        }
    }
    fields
}

/// An opaque callback declaring a whole composite needs every nested field visible.
/// This prevents a declaration of `profile` from granting its hidden child fields.
pub fn calculation_visible(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    row: &FieldMap,
    name: &str,
) -> Result<bool> {
    if crate::field_read_condition(resource, name, actor)?
        .is_some_and(|condition| !condition.matches_on(resource, row))
    {
        return Ok(false);
    }
    if let Some(calc) = resource.calculation(name)
        && calc.expr.is_custom()
        && !calc.declassify
    {
        for dependency in calc.validate_dependencies(resource)? {
            let attribute = resource
                .attribute(dependency)
                .expect("validated dependency");
            let value = row.get(dependency).cloned().unwrap_or(Value::Null);
            if crate::presentation::project_value(attribute.ty, actor, value)?.has_forbidden() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Do not invoke a hidden calculation merely to discard its result. A callback error
/// or an overflowing expression would otherwise expose a hidden operand.
pub fn apply_calculation(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    row: &mut FieldMap,
    name: &str,
    args: &FieldMap,
    trusted: bool,
) -> Result<()> {
    let visible = calculation_visible(resource, actor, row, name)?;
    if trusted || visible {
        crate::apply_named_with_args(resource, row, name, args)?;
    } else {
        row.insert(name.to_string(), Value::Null);
    }
    let projected = if visible {
        row.get(name).cloned().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    row.insert(projected_name(name), projected);
    Ok(())
}
