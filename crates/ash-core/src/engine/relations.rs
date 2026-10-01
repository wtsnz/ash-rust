use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use uuid::Uuid;

use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::policy::compile_read_filter;
use crate::resource::{RelKind, Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

pub(crate) async fn attach_relationships<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    records: &mut [R],
    loads: &[String],
) -> Result<()> {
    for name in loads {
        let rel = R::DEF.relationship(name).ok_or_else(|| {
            Error::Invalid(format!("unknown relationship `{name}` on {}", R::DEF.name))
        })?;
        let dest = (rel.destination)();
        match rel.kind {
            // Rows link on every key column, so composite and non-`id` keys work too.
            RelKind::BelongsTo | RelKind::HasMany | RelKind::HasOne => {
                let keys: Vec<Option<Vec<Value>>> = records
                    .iter()
                    .map(|record| rel.source_key(&record.to_fields()))
                    .collect();
                let first_values: BTreeSet<Value> = keys
                    .iter()
                    .flatten()
                    .filter_map(|key| key.first().cloned())
                    .collect();
                let first_column = rel.destination_columns()[0];
                let related =
                    fetch_related_values(ctx, dest, first_column, first_values.into_iter().collect())
                        .await?;
                let mut groups: BTreeMap<Vec<Value>, Vec<FieldMap>> = BTreeMap::new();
                for row in related {
                    if let Some(key) = rel.destination_key(&row) {
                        groups.entry(key).or_default().push(row);
                    }
                }
                for (record, key) in records.iter_mut().zip(keys) {
                    let attached = key
                        .and_then(|key| groups.get(&key).cloned())
                        .unwrap_or_default();
                    record.attach(name, attached)?;
                }
            }
            RelKind::ManyToMany => {
                let ids: HashSet<Uuid> = records.iter().map(Resource::id).collect();
                let through_fn = rel.through.ok_or_else(|| {
                    Error::Invalid(format!(
                        "many_to_many relationship `{}` on `{}` requires through join resource",
                        rel.name, R::DEF.name
                    ))
                })?;
                let through_def = through_fn();
                let source_on_join = rel
                    .source_attribute_on_join_resource
                    .unwrap_or(rel.source_attribute);
                let dest_on_join = rel
                    .destination_attribute_on_join_resource
                    .unwrap_or(rel.destination_attribute);

                let join_rows = fetch_related(ctx, through_def, source_on_join, &ids).await?;
                let mut source_to_dest: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
                let mut all_dest_ids: HashSet<Uuid> = HashSet::new();

                for j_row in &join_rows {
                    if let (Ok(s_id), Ok(d_id)) = (
                        required_uuid(j_row, source_on_join),
                        required_uuid(j_row, dest_on_join),
                    ) {
                        source_to_dest.entry(s_id).or_default().push(d_id);
                        all_dest_ids.insert(d_id);
                    }
                }

                let dest_rows =
                    fetch_related(ctx, dest, rel.destination_attribute, &all_dest_ids).await?;
                let by_dest_id: HashMap<Uuid, FieldMap> = dest_rows
                    .into_iter()
                    .filter_map(|row| {
                        required_uuid(&row, rel.destination_attribute)
                            .ok()
                            .map(|id| (id, row))
                    })
                    .collect();

                for record in records.iter_mut() {
                    let attached = source_to_dest
                        .get(&record.id())
                        .map(|dest_ids| {
                            dest_ids
                                .iter()
                                .filter_map(|d_id| by_dest_id.get(d_id).cloned())
                                .collect()
                        })
                        .unwrap_or_default();
                    record.attach(name, attached)?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) async fn fetch_related<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    id_field: &str,
    ids: &HashSet<Uuid>,
) -> Result<Vec<FieldMap>> {
    let values: Vec<Value> = ids.iter().copied().map(Value::Uuid).collect();
    fetch_related_values(ctx, dest, id_field, values).await
}

/// Rows of `dest` whose `field` is one of `values`, through its read policies and filters.
pub(crate) async fn fetch_related_values<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    field: &str,
    values: Vec<Value>,
) -> Result<Vec<FieldMap>> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let id_filter = Filter::In(field.to_string(), values);
    let read = dest.primary_read().ok_or(Error::NoPrimaryRead(dest.name))?;
    let policy_filter = compile_read_filter(dest, read, ctx.actor.as_ref())?;
    let filter = Filter::and(
        [Some(id_filter), policy_filter, dest.primary_read_filter()]
            .into_iter()
            .flatten(),
    );
    let mut rows = ctx
        .data
        .run_query(
            dest,
            &CompiledQuery {
                filter: Some(filter),
                tenant: ctx.tenant.clone(),
                ..CompiledQuery::default()
            },
        )
        .await?;
    for row in &mut rows {
        crate::policy::redact_fields(dest, ctx.actor.as_ref(), row)?;
    }
    Ok(rows)
}
