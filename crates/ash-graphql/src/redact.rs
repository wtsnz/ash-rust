//! Field policies, applied as Ash applies them: they hide attribute, calculation and
//! aggregate values from the requester, not relationships. A relationship loads through
//! its real keys, whoever reads it, and its destination's read policies decide what's seen.

use async_graphql::{ErrorExtensions, Value as GqlValue};

use ash_core::{Actor, FieldMap, RelationshipDef, ResourceDef, Value, hidden_fields};

/// Where a relationship key a field policy hid is kept, for loading the relationship.
fn kept(column: &str) -> String {
    format!("__graphql_key:{column}")
}

/// Where a record notes that a field policy hid `field`.
pub(crate) fn forbidden_marker(field: &str) -> String {
    format!("__graphql_forbidden:{field}")
}

/// Whether a field policy hides `field` of `record` from `actor`: as noted when the
/// record was redacted ([`redact_record`]), or, for a record redacted elsewhere, as its
/// policies say now.
pub(crate) fn is_forbidden(resource: &ResourceDef, actor: Option<&Actor>, record: &FieldMap, field: &str) -> bool {
    record.contains_key(&forbidden_marker(field))
        || hidden_fields(resource, actor, record).map_or(true, |hidden| hidden.contains(&field))
}

/// Reports the field `ctx` resolves as hidden by a field policy, as AshGraphql's
/// `forbidden_field` error at its path; the field resolves to null. (Reported here, not
/// returned: async-graphql's dynamic schemas drop a field whose resolver fails, and
/// leave its error without a path.)
pub(crate) fn report_forbidden(ctx: &async_graphql::dynamic::ResolverContext<'_>) {
    let error = async_graphql::Error::new("forbidden field").extend_with(|_, extensions| {
        extensions.set("code", "forbidden_field");
        extensions.set("short_message", "forbidden field");
        extensions.set("vars", GqlValue::Object(Default::default()));
        extensions.set("fields", GqlValue::List(Vec::new()));
    });
    let error = ctx.ctx.set_error_path(error.into_server_error(ctx.ctx.item.pos));
    ctx.ctx.add_error(error);
}

/// Hides what `actor` may not see of `record`, noting each field hidden, and keeping
/// aside the real values of any relationship keys hidden, so its relationships still
/// load.
pub(crate) fn redact_record(resource: &ResourceDef, actor: Option<&Actor>, record: &mut FieldMap) {
    if resource.field_policies.is_empty() {
        return;
    }
    let keys: Vec<(&'static str, Value)> = resource
        .relationships
        .iter()
        .flat_map(|rel| rel.source_columns())
        .filter(|column| resource.field_policies.iter().any(|policy| policy.field == *column))
        .filter_map(|column| record.get(column).map(|value| (column, value.clone())))
        .collect();
    // A policy that can't be checked hides what it guards, as every field policy then.
    let hidden = hidden_fields(resource, actor, record)
        .unwrap_or_else(|_| resource.field_policies.iter().map(|policy| policy.field).collect());
    for field in hidden {
        record.insert(field.to_string(), Value::Null);
        record.insert(forbidden_marker(field), Value::Bool(true));
    }
    for (column, value) in keys {
        if record.get(column) != Some(&value) {
            record.insert(kept(column), value);
        }
    }
}

/// The real value of `record`'s `column`, for loading a relationship through it.
pub(crate) fn key_value(record: &FieldMap, column: &str) -> Value {
    record
        .get(&kept(column))
        .or_else(|| record.get(column))
        .cloned()
        .unwrap_or(Value::Null)
}

/// What a relationship loads from: `record`'s keys for it, as they really are.
pub(crate) fn relationship_source(rel: &RelationshipDef, record: &FieldMap) -> FieldMap {
    rel.source_columns()
        .into_iter()
        .map(|column| (column.to_string(), key_value(record, column)))
        .collect()
}
