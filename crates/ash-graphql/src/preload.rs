//! Relationships loaded ahead, as AshGraphql loads them: before the fields of a query's
//! records resolve, its selection is read, and each relationship it selects is loaded for
//! every record at once, one read per relationship, nested selections in turn. The
//! relationship's field then serves the rows from its record.
//!
//! This is what a request needs, known up front, so no batch waits on a timer to gather
//! keys, and relationships beside each other load at the same time. A relationship
//! selected with arguments (`sort`, `filter`, `limit`, `offset`) is left to its field.

use ash_core::{Context, DataLayer, FieldMap, RelKind, RelationshipDef, ResourceDef, Value, load_related};
use async_graphql::SelectionField;
use futures_util::future::{BoxFuture, try_join_all};

use crate::names::camel;
use crate::redact::{redact_record, relationship_source};

/// The key a relationship's rows, selected as `response_key`, are preloaded under in its
/// record: a relationship selected twice under two aliases keeps both.
pub(crate) fn preloaded_key(relationship: &str, response_key: &str) -> String {
    format!("__graphql_preloaded:{relationship}:{response_key}")
}

/// Whether a selected field has arguments given: such a relationship is shaped by them
/// when its field resolves, not preloaded.
pub(crate) fn has_arguments(field: &SelectionField<'_>) -> async_graphql::Result<bool> {
    Ok(field.arguments()?.iter().any(|(_, value)| *value != async_graphql::Value::Null))
}

/// The fields selected under `field`, or under its `child` fields (a page's `results`, a
/// mutation's `result`), fragments included.
pub(crate) fn selected<'a>(field: SelectionField<'a>, child: Option<&str>) -> Vec<SelectionField<'a>> {
    match child {
        None => field.selection_set().collect(),
        Some(child) => field
            .selection_set()
            .filter(|f| f.name() == child)
            .flat_map(|f| f.selection_set().collect::<Vec<_>>())
            .collect(),
    }
}

/// A relationship to preload: everything selected under it, however many times it's
/// selected under the same response key.
struct Wanted<'a> {
    rel: &'static RelationshipDef,
    key: String,
    children: Vec<SelectionField<'a>>,
}

/// Loads every relationship `fields` selects on `records` of `resource`, as `ash` reads
/// them, and the relationships selected under those, in turn. `records` are as the
/// requester may see them (see [`redact_record`]); each related row is made so too.
pub(crate) fn preload<'a, D: DataLayer>(
    ash: &'a Context<D>,
    resource: &'static ResourceDef,
    fields: Vec<SelectionField<'a>>,
    records: &'a mut [FieldMap],
) -> BoxFuture<'a, async_graphql::Result<()>> {
    Box::pin(async move {
        if records.is_empty() {
            return Ok(());
        }
        let mut wanted: Vec<Wanted<'a>> = Vec::new();
        for field in fields {
            let Some(rel) = resource.relationships.iter().find(|rel| camel(rel.name) == field.name()) else {
                continue;
            };
            if has_arguments(&field)? {
                continue;
            }
            let key = preloaded_key(rel.name, field.alias().unwrap_or(field.name()));
            let children: Vec<SelectionField<'a>> = field.selection_set().collect();
            match wanted.iter_mut().find(|w| w.key == key) {
                Some(same) => same.children.extend(children),
                None => wanted.push(Wanted { rel, key, children }),
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }

        let records_ref: &[FieldMap] = records;
        let loaded = try_join_all(wanted.into_iter().map(|Wanted { rel, key, children }| async move {
            let destination = (rel.destination)();
            let sources: Vec<FieldMap> = records_ref.iter().map(|record| relationship_source(rel, record)).collect();
            let related = load_related(ash, resource, rel.name, &sources)
                .await
                .map_err(|e| async_graphql::Error::new(e.to_string()))?;
            // Every related row once, as the requester may see it, so what's selected
            // under it loads in one go too.
            let counts: Vec<usize> = related.iter().map(Vec::len).collect();
            let mut rows: Vec<FieldMap> = related.into_iter().flatten().collect();
            for row in &mut rows {
                redact_record(destination, ash.actor.as_ref(), row);
            }
            preload(ash, destination, children, &mut rows).await?;

            let to_one = matches!(rel.kind, RelKind::BelongsTo | RelKind::HasOne);
            let mut rows = rows.into_iter();
            let values: Vec<Value> = counts
                .into_iter()
                .map(|count| {
                    // All of this record's rows, so none is left over for the next.
                    let mine: Vec<Value> = rows.by_ref().take(count).map(Value::Map).collect();
                    if to_one {
                        mine.into_iter().next().unwrap_or(Value::Null)
                    } else {
                        Value::Array(mine)
                    }
                })
                .collect();
            Ok::<_, async_graphql::Error>((key, values))
        }))
        .await?;

        for (key, values) in loaded {
            for (record, value) in records.iter_mut().zip(values) {
                record.insert(key.clone(), value);
            }
        }
        Ok(())
    })
}
