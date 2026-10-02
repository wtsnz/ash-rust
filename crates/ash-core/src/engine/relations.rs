use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use uuid::Uuid;

use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::{RelKind, Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

pub(crate) async fn attach_relationships<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    records: &mut [R],
    loads: &[String],
) -> Result<()> {
    if loads.is_empty() || records.is_empty() {
        return Ok(());
    }
    let sources: Vec<FieldMap> = records.iter().map(Resource::to_fields).collect();
    for name in loads {
        let related = load_related(ctx, &R::DEF, name, &sources).await?;
        for (record, rows) in records.iter_mut().zip(related) {
            record.attach(name, rows)?;
        }
    }
    Ok(())
}

/// The rows related to each of `sources` (records of `resource`) through
/// `relationship`, in the same order.
///
/// Destination rows, and join rows for `many_to_many`, are read as that resource's
/// default read sees them in `ctx`: its preparations, the actor's read policies, and
/// the tenant, with fields the actor may not read redacted. The read's sort orders each
/// source's rows, and its limit and offset page them per source. Rows link on every key
/// column, so composite and non-`id` keys work. Typed relationship loads and GraphQL
/// both load through it.
pub async fn load_related<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    relationship: &str,
    sources: &[FieldMap],
) -> Result<Vec<Vec<FieldMap>>> {
    load_related_query(ctx, resource, relationship, sources, &RelatedQuery::default()).await
}

/// How a relationship's rows are read, as a query given to `Ash.Query.load` shapes
/// them: a filter and a sort on the destination, and a limit and offset that page each
/// source's rows. What it leaves out, the destination's default read decides.
#[derive(Clone, Debug, Default)]
pub struct RelatedQuery {
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    /// The destination's attributes to read, as Ash's `select` (`None`: every one). The
    /// keys the load links rows to their sources by are read whatever it says.
    pub select: Option<Vec<String>>,
    /// The destination's aggregates and calculations to load with its rows.
    pub aggregates: Vec<String>,
    pub calculations: Vec<String>,
}

impl RelatedQuery {
    /// This query, reading `keys` too if it selects attributes.
    fn reading(&self, keys: &[&str]) -> RelatedQuery {
        let mut query = self.clone();
        if let Some(select) = &mut query.select {
            for key in keys {
                if !select.iter().any(|name| name == key) {
                    select.push(key.to_string());
                }
            }
        }
        query
    }
}

/// [`load_related`], shaped by `query`: still one read of the destination for every
/// source, as Ash loads a relationship without a lateral join, each source's rows then
/// paged by the limit and offset.
pub async fn load_related_query<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ResourceDef,
    relationship: &str,
    sources: &[FieldMap],
    query: &RelatedQuery,
) -> Result<Vec<Vec<FieldMap>>> {
    let rel = resource.relationship(relationship).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown relationship `{relationship}` on {}",
            resource.name
        ))
    })?;
    let dest = (rel.destination)();
    match rel.kind {
        RelKind::BelongsTo | RelKind::HasMany | RelKind::HasOne => {
            let keys: Vec<Option<Vec<Value>>> = sources
                .iter()
                .map(|source| rel.source_key(source))
                .collect();
            let first_values: BTreeSet<Value> = keys
                .iter()
                .flatten()
                .filter_map(|key| key.first().cloned())
                .collect();
            let first_column = rel.destination_columns()[0];
            let query = query.reading(&rel.destination_columns());
            let mut related =
                fetch_related_values(ctx, dest, first_column, first_values.into_iter().collect(), &query)
                    .await?;
            // Each row moves into its key's group, and each group is paged once.
            let mut groups: BTreeMap<Vec<Value>, Vec<FieldMap>> = BTreeMap::new();
            for row in std::mem::take(&mut related.rows) {
                if let Some(key) = rel.destination_key(&row) {
                    groups.entry(key).or_default().push(row);
                }
            }
            for rows in groups.values_mut() {
                *rows = related.page(std::mem::take(rows));
            }
            // Sources sharing a key (posts by one author) each get its rows: copies for
            // all but the last, which takes them.
            let mut seen = BTreeSet::new();
            let last: Vec<bool> = keys.iter().rev().map(|key| key.as_ref().is_some_and(|key| seen.insert(key))).collect();
            Ok(keys
                .iter()
                .zip(last.into_iter().rev())
                .map(|(key, last)| match key {
                    Some(key) if last => groups.remove(key).unwrap_or_default(),
                    Some(key) => groups.get(key).cloned().unwrap_or_default(),
                    None => Vec::new(),
                })
                .collect())
        }
        RelKind::ManyToMany => {
            let source_ids: Vec<Option<Uuid>> = sources
                .iter()
                .map(|source| required_uuid(source, rel.source_attribute).ok())
                .collect();
            let through_fn = rel.through.ok_or_else(|| {
                Error::Invalid(format!(
                    "many_to_many relationship `{}` on `{}` requires through join resource",
                    rel.name, resource.name
                ))
            })?;
            let through_def = through_fn();
            let source_on_join = rel
                .source_attribute_on_join_resource
                .unwrap_or(rel.source_attribute);
            let dest_on_join = rel
                .destination_attribute_on_join_resource
                .unwrap_or(rel.destination_attribute);

            let ids: HashSet<Uuid> = source_ids.iter().flatten().copied().collect();
            let join_rows = fetch_related(ctx, through_def, source_on_join, &ids, &RelatedQuery::default()).await?;
            let mut source_to_dest: HashMap<Uuid, HashSet<Uuid>> = HashMap::new();
            let mut all_dest_ids: HashSet<Uuid> = HashSet::new();
            for j_row in &join_rows.rows {
                if let (Ok(s_id), Ok(d_id)) = (
                    required_uuid(j_row, source_on_join),
                    required_uuid(j_row, dest_on_join),
                ) {
                    source_to_dest.entry(s_id).or_default().insert(d_id);
                    all_dest_ids.insert(d_id);
                }
            }

            // Each source's rows keep the destination read's order.
            let related =
                fetch_related(ctx, dest, rel.destination_attribute, &all_dest_ids, &query.reading(&[rel.destination_attribute]))
                    .await?;
            Ok(source_ids
                .into_iter()
                .map(|source_id| {
                    let Some(linked) = source_id.and_then(|id| source_to_dest.get(&id)) else {
                        return Vec::new();
                    };
                    related.page(
                        related
                            .rows
                            .iter()
                            .filter(|row| {
                                required_uuid(row, rel.destination_attribute)
                                    .is_ok_and(|id| linked.contains(&id))
                            })
                            .cloned()
                            .collect(),
                    )
                })
                .collect())
        }
    }
}

/// Rows read for many sources at once, and the paging their read applies to each
/// source's share of them.
pub(crate) struct Related {
    pub(crate) rows: Vec<FieldMap>,
    offset: usize,
    limit: Option<usize>,
}

impl Related {
    /// One source's rows, paged as its read pages them.
    fn page(&self, rows: Vec<FieldMap>) -> Vec<FieldMap> {
        if self.offset == 0 && self.limit.is_none_or(|limit| rows.len() <= limit) {
            return rows;
        }
        rows.into_iter()
            .skip(self.offset)
            .take(self.limit.unwrap_or(usize::MAX))
            .collect()
    }
}

pub(crate) async fn fetch_related<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    id_field: &str,
    ids: &HashSet<Uuid>,
    shape: &RelatedQuery,
) -> Result<Related> {
    let values: Vec<Value> = ids.iter().copied().map(Value::Uuid).collect();
    fetch_related_values(ctx, dest, id_field, values, shape).await
}

/// Rows of `dest` whose `field` is one of `values`, as its default read sees them,
/// shaped by `shape`.
pub(crate) async fn fetch_related_values<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    field: &str,
    values: Vec<Value>,
    shape: &RelatedQuery,
) -> Result<Related> {
    let mut related = Related {
        rows: Vec::new(),
        offset: 0,
        limit: None,
    };
    if values.is_empty() {
        return Ok(related);
    }
    let read = dest.default_read();
    // A load reads the destination as its default read would, in the context's tenant.
    let mut query = super::read::scope_read(
        dest,
        read,
        ctx.actor.as_ref(),
        &FieldMap::new(),
        CompiledQuery {
            filter: Some(Filter::and([Some(Filter::In(field.to_string(), values)), shape.filter.clone()].into_iter().flatten())),
            sort: shape.sort.clone(),
            limit: shape.limit,
            offset: shape.offset,
            select: shape.select.clone(),
            aggregates: shape.aggregates.clone(),
            calculations: shape.calculations.clone(),
            tenant: ctx.tenant.clone(),
            ..CompiledQuery::default()
        },
    )?;
    // The batch serves many sources, so the read's paging is applied to each source's
    // rows rather than to the batch.
    related.offset = query.offset.take().unwrap_or(0);
    related.limit = query.limit.take();
    related.rows = ctx.data.run_query(dest, &query).await?;
    for row in &mut related.rows {
        crate::policy::redact_fields(dest, ctx.actor.as_ref(), row)?;
    }
    Ok(related)
}
