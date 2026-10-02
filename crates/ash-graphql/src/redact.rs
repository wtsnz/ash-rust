//! Field policies, applied as Ash applies them: they hide attribute, calculation and
//! aggregate values from the requester, not relationships. A relationship loads through
//! its real keys, whoever reads it, and its destination's read policies decide what's seen.

use ash_core::{Actor, FieldMap, RelationshipDef, ResourceDef, Value, redact_fields};

/// Where a relationship key a field policy hid is kept, for loading the relationship.
fn kept(column: &str) -> String {
    format!("__graphql_key:{column}")
}

/// Hides what `actor` may not see of `record`, keeping aside the real values of any
/// relationship keys hidden, so its relationships still load.
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
    let _ = redact_fields(resource, actor, record);
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
