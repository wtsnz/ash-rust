use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, PerKey, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::{RelKind, Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_pk};

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
    /// The arguments each calculation that takes them is loaded with.
    pub calculation_args: std::collections::HashMap<String, FieldMap>,
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

/// [`load_related`], shaped by `query`: one read of the destination for every source.
/// A limit or offset pages each source's rows: in the read itself, once per source, where
/// the data layer can join laterally, as AshPostgres loads a relationship; else in memory,
/// as Ash does for a data layer that can't.
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
    // Each source's key, and each key's rows.
    let (keys, groups) = match rel.kind {
        RelKind::BelongsTo | RelKind::HasMany | RelKind::HasOne => {
            let keys: Vec<Option<Vec<Value>>> = sources.iter().map(|source| rel.source_key(source)).collect();
            let columns = rel.destination_columns();
            let first_values: Vec<Value> = keys
                .iter()
                .flatten()
                .filter_map(|key| key.first().cloned())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            // No source has a key: nothing to read, nor to scope a read for.
            if first_values.is_empty() {
                return Ok(vec![Vec::new(); sources.len()]);
            }
            let read = related_read(ctx, dest, None, &query.reading(&columns))?;
            let mut groups: BTreeMap<Vec<Value>, Vec<FieldMap>> = BTreeMap::new();
            let by = PerKey::Attribute(columns[0]);
            if columns.len() == 1 && paged(&read) && ctx.data.can_run_query_per_key(dest, &by) {
                let per_key = read_per_key(ctx, dest, &read, &by, &first_values).await?;
                for (value, rows) in first_values.into_iter().zip(per_key) {
                    groups.insert(vec![value], rows);
                }
            } else {
                let mut related = read_batch(ctx, dest, columns[0], first_values, read).await?;
                // Each row moves into its key's group, and each group is paged once.
                for row in std::mem::take(&mut related.rows) {
                    if let Some(key) = rel.destination_key(&row) {
                        groups.entry(key).or_default().push(row);
                    }
                }
                for rows in groups.values_mut() {
                    *rows = related.page(std::mem::take(rows));
                }
            }
            (keys, groups)
        }
        RelKind::ManyToMany => {
            let keys: Vec<Option<Vec<Value>>> = sources
                .iter()
                .map(|source| required_pk(source, rel.source_attribute).ok().map(|id| vec![id]))
                .collect();
            let source_ids: Vec<Value> =
                keys.iter().flatten().flatten().cloned().collect::<BTreeSet<_>>().into_iter().collect();
            let through = rel.through.ok_or_else(|| {
                Error::Invalid(format!(
                    "many_to_many relationship `{}` on `{}` requires through join resource",
                    rel.name, resource.name
                ))
            })?();
            let source_on_join = rel.source_attribute_on_join_resource.unwrap_or(rel.source_attribute);
            let dest_on_join = rel.destination_attribute_on_join_resource.unwrap_or(rel.destination_attribute);
            if source_ids.is_empty() {
                return Ok(vec![Vec::new(); sources.len()]);
            }
            let read = related_read(ctx, dest, None, &query.reading(&[rel.destination_attribute]))?;
            let mut groups: BTreeMap<Vec<Value>, Vec<FieldMap>> = BTreeMap::new();
            // The join rows as the actor reads them, linking each source to its rows.
            let joins = related_read(ctx, through, None, &RelatedQuery::default())?;
            let by = PerKey::Through {
                resource: through,
                filter: joins.filter.as_ref(),
                source: source_on_join,
                destination: dest_on_join,
                attribute: rel.destination_attribute,
            };
            if paged(&read) && ctx.data.can_run_query_per_key(dest, &by) {
                let per_key = read_per_key(ctx, dest, &read, &by, &source_ids).await?;
                for (value, rows) in source_ids.into_iter().zip(per_key) {
                    groups.insert(vec![value], rows);
                }
            } else {
                let join_rows = read_batch(ctx, through, source_on_join, source_ids, joins.clone()).await?;
                let mut source_to_dest: HashMap<Value, HashSet<Value>> = HashMap::new();
                let mut all_dest_ids: BTreeSet<Value> = BTreeSet::new();
                for j_row in &join_rows.rows {
                    if let (Ok(s_id), Ok(d_id)) = (required_pk(j_row, source_on_join), required_pk(j_row, dest_on_join)) {
                        source_to_dest.entry(s_id).or_default().insert(d_id.clone());
                        all_dest_ids.insert(d_id);
                    }
                }
                // Each source's rows keep the destination read's order.
                let related =
                    read_batch(ctx, dest, rel.destination_attribute, all_dest_ids.into_iter().collect(), read).await?;
                for (source_id, linked) in source_to_dest {
                    let rows = related
                        .rows
                        .iter()
                        .filter(|row| required_pk(row, rel.destination_attribute).is_ok_and(|id| linked.contains(&id)))
                        .cloned()
                        .collect();
                    groups.insert(vec![source_id], related.page(rows));
                }
            }
            (keys, groups)
        }
    };
    Ok(distribute(keys, groups))
}

/// Each source's rows, by its key. Sources sharing a key (posts by one author) each get
/// its rows: copies for all but the last, which takes them.
fn distribute(keys: Vec<Option<Vec<Value>>>, mut groups: BTreeMap<Vec<Value>, Vec<FieldMap>>) -> Vec<Vec<FieldMap>> {
    let mut seen = BTreeSet::new();
    let last: Vec<bool> = keys.iter().rev().map(|key| key.as_ref().is_some_and(|key| seen.insert(key))).collect();
    keys.iter()
        .zip(last.into_iter().rev())
        .map(|(key, last)| match key {
            Some(key) if last => groups.remove(key).unwrap_or_default(),
            Some(key) => groups.get(key).cloned().unwrap_or_default(),
            None => Vec::new(),
        })
        .collect()
}

/// Whether a read pages its rows.
fn paged(read: &CompiledQuery) -> bool {
    read.limit.is_some() || read.offset.is_some_and(|offset| offset > 0)
}

/// `read` once for each of `keys`, as the data layer joins it laterally, its rows as the
/// actor may see them.
async fn read_per_key<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    read: &CompiledQuery,
    by: &PerKey<'_>,
    keys: &[Value],
) -> Result<Vec<Vec<FieldMap>>> {
    if keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut per_key = ctx.data.run_query_per_key(dest, read, by, keys).await?;
    for row in per_key.iter_mut().flatten() {
        crate::policy::redact_fields(dest, ctx.actor.as_ref(), row)?;
    }
    Ok(per_key)
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

/// The read of `dest` a relationship load runs: as its default read sees it in the
/// context's tenant, shaped by `shape`, and filtered by `filter` too.
fn related_read<D>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    filter: Option<Filter>,
    shape: &RelatedQuery,
) -> Result<CompiledQuery> {
    super::read::scope_read(
        dest,
        dest.default_read(),
        ctx.actor.as_ref(),
        &FieldMap::new(),
        CompiledQuery {
            filter: match Filter::and([filter, shape.filter.clone()].into_iter().flatten()) {
                Filter::True => None,
                filter => Some(filter),
            },
            sort: shape.sort.clone(),
            limit: shape.limit,
            offset: shape.offset,
            select: shape.select.clone(),
            aggregates: shape.aggregates.clone(),
            calculations: shape.calculations.clone(),
            calculation_args: shape.calculation_args.clone(),
            tenant: ctx.tenant.clone(),
            ..CompiledQuery::default()
        },
    )
}

/// `read` of the rows whose `field` is one of `values`, all at once. The batch serves
/// many sources, so the read's paging is taken out, for each source's rows.
async fn read_batch<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    field: &str,
    values: Vec<Value>,
    mut read: CompiledQuery,
) -> Result<Related> {
    let mut related = Related {
        rows: Vec::new(),
        offset: read.offset.take().unwrap_or(0),
        limit: read.limit.take(),
    };
    if values.is_empty() {
        return Ok(related);
    }
    let by_key = Filter::In(field.to_string(), values);
    read.filter = Some(match read.filter.take() {
        Some(filter) => Filter::and([by_key, filter]),
        None => by_key,
    });
    related.rows = ctx.data.run_query(dest, &read).await?;
    for row in &mut related.rows {
        crate::policy::redact_fields(dest, ctx.actor.as_ref(), row)?;
    }
    Ok(related)
}
