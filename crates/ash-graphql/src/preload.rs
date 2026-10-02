//! Relationships loaded ahead, as AshGraphql loads them: before the fields of a query's
//! records resolve, its selection is read, and each relationship it selects is loaded for
//! every record at once, one read per relationship, nested selections in turn. The
//! relationship's field then serves the rows from its record.
//!
//! This is what a request needs, known up front, so no batch waits on a timer to gather
//! keys, and relationships beside each other load at the same time. A relationship
//! selected with arguments (`sort`, `filter`, `limit`, `offset`) loads shaped by them,
//! as AshGraphql loads it with the related query they build: still one read, each
//! record's rows then limited and offset.

use ash_core::{
    CompiledQuery, Context, DataLayer, FieldMap, Filter, RelKind, RelatedQuery, RelationshipDef, ResourceDef, Value,
    field_policy_fields, load_related_query,
};
use async_graphql::{SelectionField, Value as GqlValue};
use futures_util::future::{BoxFuture, try_join_all};

use crate::filter::parse_resource_filter;
use crate::names::camel;
use crate::redact::{redact_record, relationship_source};
use crate::sort::parse_resource_sort;

/// The key a relationship's rows, selected as `field`, are preloaded under in its record:
/// by response key, so a relationship selected under two aliases keeps both, and by the
/// arguments that shape them, so a relationship selected under the same key with others
/// (under two aliases of the records' own field) keeps both too.
pub(crate) fn preloaded_key(relationship: &str, field: &SelectionField<'_>) -> async_graphql::Result<String> {
    let mut key = format!("__graphql_preloaded:{relationship}:{}", field.alias().unwrap_or(field.name()));
    for (name, value) in field.arguments()? {
        if !matches!(value, GqlValue::Null) {
            key.push_str(&format!(":{name}={value}"));
        }
    }
    Ok(key)
}

/// The related query a to-many relationship field's arguments build, as AshGraphql's:
/// `filter` and `sort` on the destination, and `limit` and `offset` paging each
/// record's rows. Arguments not given (or null) leave the destination's read to decide.
pub(crate) fn related_query<'v>(
    destination: &'static ResourceDef,
    arguments: impl IntoIterator<Item = (&'v str, &'v GqlValue)>,
) -> async_graphql::Result<RelatedQuery> {
    let mut query = RelatedQuery::default();
    let number = |value: &GqlValue| match value {
        GqlValue::Number(n) => n.as_i64().map(|n| n.max(0) as usize),
        _ => None,
    };
    for (name, value) in arguments {
        if matches!(value, GqlValue::Null) {
            continue;
        }
        match name {
            "filter" => query.filter = Some(parse_resource_filter(destination, value)?),
            "sort" => query.sort = parse_resource_sort(destination, value)?,
            "limit" => query.limit = number(value),
            "offset" => query.offset = number(value),
            _ => {}
        }
    }
    Ok(query)
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

/// What a read of a resource loads for the fields selected on its records, as
/// AshGraphql's `select_fields` and `load_fields` decide: the attributes selected, the
/// keys of the relationships selected, and the aggregates and calculations selected;
/// and, whatever is selected, the primary key and the fields its field policies check.
#[derive(Debug, Default)]
pub(crate) struct Load {
    pub select: Vec<String>,
    pub aggregates: Vec<String>,
    pub calculations: Vec<String>,
    /// A calculation only Rust can compute is selected, which reads whatever of the
    /// record it likes: every attribute is read.
    pub every_attribute: bool,
}

impl Load {
    /// What `fields` selected on records of `resource` need loaded, and `also` (the
    /// fields a read sorts by, which its keysets hold).
    pub(crate) fn of<'s>(
        resource: &'static ResourceDef,
        fields: &[SelectionField<'_>],
        also: impl IntoIterator<Item = &'s str>,
    ) -> Load {
        let selected: Vec<&str> = fields.iter().map(|field| field.name()).collect();
        let is_selected = |name: &str| selected.iter().any(|field| *field == camel(name));
        let mut load = Load::default();
        for attr in resource.attributes.iter().filter(|attr| attr.primary_key || is_selected(attr.name)) {
            load.add(resource, attr.name);
        }
        for agg in resource.aggregates.iter().filter(|agg| is_selected(agg.name)) {
            load.add(resource, agg.name);
        }
        for calc in resource.calculations.iter().filter(|calc| is_selected(calc.name)) {
            if calc.expr.is_custom() {
                load.every_attribute = true;
            } else {
                load.add(resource, calc.name);
            }
        }
        for rel in resource.relationships.iter().filter(|rel| is_selected(rel.name)) {
            for column in rel.source_columns() {
                load.add(resource, column);
            }
        }
        for field in field_policy_fields(resource).into_iter().chain(also) {
            load.add(resource, field);
        }
        load
    }

    /// Loads `field` of `resource`: an attribute, aggregate or calculation.
    fn add(&mut self, resource: &ResourceDef, field: &str) {
        let list = if resource.attribute(field).is_some() {
            &mut self.select
        } else if resource.aggregate(field).is_some() {
            &mut self.aggregates
        } else if resource.calculation(field).is_some() {
            &mut self.calculations
        } else {
            return;
        };
        if !list.iter().any(|name| name == field) {
            list.push(field.to_string());
        }
    }

    /// `query`, reading only this.
    pub(crate) fn onto(self, query: CompiledQuery) -> CompiledQuery {
        let mut query = query;
        query.select = (!self.every_attribute).then_some(self.select);
        query.aggregates.extend(self.aggregates);
        query.calculations.extend(self.calculations);
        query
    }

    /// `query`, loading only this of the related rows.
    fn onto_related(self, query: RelatedQuery) -> RelatedQuery {
        RelatedQuery {
            select: (!self.every_attribute).then_some(self.select),
            aggregates: self.aggregates,
            calculations: self.calculations,
            ..query
        }
    }
}

/// Loads the aggregates and calculations `fields` select on `records`, written records of
/// `resource`, as AshGraphql loads a mutation's result: one read of them all, by key, as
/// the actor reads them. Load them before redacting the records, so field policies apply
/// to what loads too.
pub(crate) async fn load_selected<D: DataLayer>(
    ash: &Context<D>,
    resource: &'static ResourceDef,
    fields: &[SelectionField<'_>],
    records: &mut [FieldMap],
) -> async_graphql::Result<()> {
    let load = Load::of(resource, fields, []);
    if records.is_empty() || (load.aggregates.is_empty() && load.calculations.is_empty()) {
        return Ok(());
    }
    let Some(pk) = resource.primary_key().map(|attr| attr.name) else {
        return Ok(());
    };
    let ids: Vec<Value> = records.iter().filter_map(|record| record.get(pk).cloned()).collect();
    let query = CompiledQuery {
        filter: Some(Filter::In(pk.to_string(), ids)),
        select: Some(Vec::new()),
        aggregates: load.aggregates,
        calculations: load.calculations,
        tenant: ash.tenant.clone(),
        actor: ash.actor.clone(),
        ..CompiledQuery::default()
    };
    let loaded = ash.data.run_query(resource, &query).await.map_err(|e| async_graphql::Error::new(e.to_string()))?;
    for record in records.iter_mut() {
        if let Some(row) = loaded.iter().find(|row| row.get(pk) == record.get(pk)) {
            for name in query.aggregates.iter().chain(&query.calculations) {
                if let Some(value) = row.get(name) {
                    record.insert(name.clone(), value.clone());
                }
            }
        }
    }
    Ok(())
}

/// A relationship to preload: everything selected under it, however many times it's
/// selected under the same response key.
struct Wanted<'a> {
    rel: &'static RelationshipDef,
    key: String,
    /// The related query its arguments build, which its key holds.
    query: RelatedQuery,
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
            let key = preloaded_key(rel.name, &field)?;
            let children: Vec<SelectionField<'a>> = field.selection_set().collect();
            match wanted.iter_mut().find(|w| w.key == key) {
                Some(same) => same.children.extend(children),
                None => {
                    let arguments = field.arguments()?;
                    let query = related_query(
                        (rel.destination)(),
                        arguments.iter().map(|(name, value)| (name.as_str(), value)),
                    )?;
                    wanted.push(Wanted { rel, key, query, children });
                }
            }
        }
        if wanted.is_empty() {
            return Ok(());
        }

        let records_ref: &[FieldMap] = records;
        let loaded = try_join_all(wanted.into_iter().map(|Wanted { rel, key, query, children }| async move {
            let destination = (rel.destination)();
            let sort: Vec<String> = query.sort.iter().map(|sort| sort.field.clone()).collect();
            let query = Load::of(destination, &children, sort.iter().map(String::as_str)).onto_related(query);
            let sources: Vec<FieldMap> = records_ref.iter().map(|record| relationship_source(rel, record)).collect();
            let related = load_related_query(ash, resource, rel.name, &sources, &query)
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
