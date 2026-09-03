//! HashMap-backed data layer, in the spirit of Ash.DataLayer.Ets.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::ready;
use std::sync::{Arc, Mutex, MutexGuard};

use ash_core::{
    AggregateFilter, AggregateKind, CompiledQuery, DataLayer, Error, FieldMap, ResourceDef, Result,
    SchemaSupport, TransactionSupport, Value, apply_named,
};
use uuid::Uuid;

#[derive(Clone, Debug, Default)]
pub struct Memory {
    tables: Arc<Mutex<HashMap<String, HashMap<Uuid, FieldMap>>>>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn from_tables(tables: HashMap<String, HashMap<Uuid, FieldMap>>) -> Self {
        Self {
            tables: Arc::new(Mutex::new(tables)),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, HashMap<String, HashMap<Uuid, FieldMap>>>> {
        self.tables
            .lock()
            .map_err(|_| Error::DataLayer("memory store lock poisoned".into()))
    }
}

fn check_identities(
    resource: &ResourceDef,
    table: &HashMap<Uuid, FieldMap>,
    id: Uuid,
    fields: &FieldMap,
) -> Result<()> {
    for ident in resource.identities {
        for (existing_id, row) in table.iter() {
            if *existing_id == id {
                continue;
            }
            let matches_all = ident.keys.iter().all(|k| {
                let new_val = fields.get(*k);
                let existing_val = row.get(*k);
                match (new_val, existing_val) {
                    (Some(a), Some(b)) if !a.is_null() && !b.is_null() => a == b,
                    _ => false,
                }
            });
            if matches_all {
                return Err(Error::IdentityConflict {
                    identity: ident.name,
                    fields: ident.keys.iter().map(|s| s.to_string()).collect(),
                    message: ident
                        .message
                        .unwrap_or("record with this identity already exists")
                        .to_string(),
                });
            }
        }
    }
    Ok(())
}

impl DataLayer for Memory {
    fn create(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.entry(resource.name.to_string()).or_default();
            if table.contains_key(&id) {
                return Err(Error::DataLayer(format!(
                    "duplicate id {id} in {}",
                    resource.name
                )));
            }
            check_identities(resource, table, id, &fields)?;
            table.insert(id, fields.clone());
            Ok(fields)
        })())
    }

    fn update(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.get_mut(resource.name).ok_or(Error::NotFound)?;
            let mut current_row = table.get(&id).ok_or(Error::NotFound)?.clone();

            if let Some(v_attr) = resource.optimistic_lock_attribute()
                && let Some(Value::Int(new_v)) = fields.get(v_attr)
            {
                let expected_v = new_v - 1;
                let actual_v = current_row
                    .get(v_attr)
                    .and_then(|v| match v {
                        Value::Int(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(1);
                if actual_v != expected_v {
                    return Err(Error::StaleRecord {
                        resource: resource.name,
                        id,
                    });
                }
            }

            current_row.extend(fields);
            check_identities(resource, table, id, &current_row)?;
            table.insert(id, current_row.clone());
            Ok(current_row)
        })())
    }

    fn upsert(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
        identity: &ash_core::IdentityDef,
        update_fields: &[String],
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.entry(resource.name.to_string()).or_default();
            let existing_entry = table
                .iter()
                .find(|(_, row)| {
                    identity.keys.iter().all(|k| {
                        let new_val = fields.get(*k);
                        let existing_val = row.get(*k);
                        match (new_val, existing_val) {
                            (Some(a), Some(b)) if !a.is_null() && !b.is_null() => a == b,
                            _ => false,
                        }
                    })
                })
                .map(|(existing_id, row)| (*existing_id, row.clone()));

            if let Some((existing_id, mut existing_row)) = existing_entry {
                let to_update: Vec<(String, Value)> = if update_fields.is_empty() {
                    fields
                        .into_iter()
                        .filter(|(k, _)| !identity.keys.contains(&k.as_str()))
                        .collect()
                } else {
                    update_fields
                        .iter()
                        .filter_map(|k| fields.get(k).map(|v| (k.clone(), v.clone())))
                        .collect()
                };
                for (k, v) in to_update {
                    existing_row.insert(k, v);
                }
                table.insert(existing_id, existing_row.clone());
                Ok(existing_row)
            } else {
                check_identities(resource, table, id, &fields)?;
                table.insert(id, fields.clone());
                Ok(fields)
            }
        })())
    }

    fn destroy(&self, resource: &ResourceDef, id: Uuid) -> impl Future<Output = Result<()>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.get_mut(resource.name).ok_or(Error::NotFound)?;
            table.remove(&id).ok_or(Error::NotFound)?;
            Ok(())
        })())
    }

    fn bulk_create(
        &self,
        resource: &ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.entry(resource.name.to_string()).or_default();
            let mut results = Vec::with_capacity(rows.len());
            for (id, fields) in rows {
                if table.contains_key(&id) {
                    return Err(Error::DataLayer(format!(
                        "duplicate id {id} in {}",
                        resource.name
                    )));
                }
                check_identities(resource, table, id, &fields)?;
                table.insert(id, fields.clone());
                results.push(fields);
            }
            Ok(results)
        })())
    }

    fn bulk_destroy(
        &self,
        resource: &ResourceDef,
        ids: &[Uuid],
    ) -> impl Future<Output = Result<()>> + Send {
        ready((|| {
            let mut tables = self.lock()?;
            let table = tables.get_mut(resource.name).ok_or(Error::NotFound)?;
            for id in ids {
                table.remove(id);
            }
            Ok(())
        })())
    }

    fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let tables = self.lock()?;
            let mut rows: Vec<FieldMap> = tables
                .get(resource.name)
                .map(|table| table.values().cloned().collect())
                .unwrap_or_default();

            let needed = needed_calculations(resource, query);
            for row in &mut rows {
                for name in &needed {
                    apply_named(resource, row, name)?;
                }
            }

            let needed_aggs = needed_aggregates(resource, query);
            apply_aggregates(&tables, resource, &mut rows, &needed_aggs)?;

            if let Some(filter) = &query.filter {
                rows.retain(|row| filter.matches(row));
            }

            if !query.sort.is_empty() {
                rows.sort_by(|left, right| {
                    let mut order = Ordering::Equal;
                    for sort in &query.sort {
                        let left_value = left.get(&sort.field).cloned().unwrap_or(Value::Null);
                        let right_value = right.get(&sort.field).cloned().unwrap_or(Value::Null);
                        order = left_value.cmp(&right_value);
                        if sort.descending {
                            order = order.reverse();
                        }
                        if order != Ordering::Equal {
                            break;
                        }
                    }
                    order
                });
            }

            let offset = query.offset.unwrap_or(0);
            if offset >= rows.len() {
                rows.clear();
            } else {
                rows = rows.split_off(offset);
            }

            if let Some(limit) = query.limit {
                rows.truncate(limit);
            }

            for row in &mut rows {
                strip_unrequested_calculations(resource, query, row);
                strip_unrequested_aggregates(resource, query, row);
            }

            Ok(rows)
        })())
    }
}

fn needed_calculations<'a>(resource: &'a ResourceDef, query: &'a CompiledQuery) -> Vec<&'a str> {
    let mut names = Vec::new();
    if let Some(filter) = &query.filter {
        filter.collect_fields(&mut names);
    }
    for sort in &query.sort {
        names.push(sort.field.as_str());
    }
    for name in &query.calculations {
        names.push(name.as_str());
    }
    names.retain(|name| resource.calculation(name).is_some());
    names.sort_unstable();
    names.dedup();
    names
}

fn strip_unrequested_calculations(
    resource: &ResourceDef,
    query: &CompiledQuery,
    row: &mut FieldMap,
) {
    for calc in resource.calculations {
        if !query.calculations.iter().any(|name| name == calc.name) {
            row.remove(calc.name);
        }
    }
}

fn needed_aggregates<'a>(resource: &'a ResourceDef, query: &'a CompiledQuery) -> Vec<&'a str> {
    let mut names = Vec::new();
    if let Some(filter) = &query.filter {
        filter.collect_fields(&mut names);
    }
    for sort in &query.sort {
        names.push(sort.field.as_str());
    }
    for name in &query.aggregates {
        names.push(name.as_str());
    }
    names.retain(|name| resource.aggregate(name).is_some());
    names.sort_unstable();
    names.dedup();
    names
}

fn strip_unrequested_aggregates(
    resource: &ResourceDef,
    query: &CompiledQuery,
    row: &mut FieldMap,
) {
    for agg in resource.aggregates {
        if !query.aggregates.iter().any(|name| name == agg.name) {
            row.remove(agg.name);
        }
    }
}

fn apply_aggregates(
    tables: &HashMap<String, HashMap<Uuid, FieldMap>>,
    resource: &ResourceDef,
    rows: &mut [FieldMap],
    needed: &[&str],
) -> Result<()> {
    for agg_name in needed {
        let agg = resource.aggregate(agg_name).ok_or_else(|| {
            Error::Invalid(format!("unknown aggregate `{agg_name}` on {}", resource.name))
        })?;
        let rel = resource.relationship(agg.relationship).ok_or_else(|| {
            Error::Invalid(format!(
                "unknown relationship `{}` in aggregate `{}` on {}",
                agg.relationship, agg.name, resource.name
            ))
        })?;
        let dest = (rel.destination)();
        let dest_rows: Vec<&FieldMap> = tables
            .get(dest.name)
            .map(|t| t.values().collect())
            .unwrap_or_default();

        for row in rows.iter_mut() {
            let source_val = row.get(rel.source_attribute).cloned().unwrap_or(Value::Null);
            if source_val.is_null() {
                let default_val = match agg.kind {
                    AggregateKind::Count => Value::Int(0),
                    AggregateKind::Exists => Value::Bool(false),
                    AggregateKind::First { .. } => Value::Null,
                    AggregateKind::Sum { .. } => Value::Int(0),
                };
                row.insert(agg.name.to_string(), default_val);
                continue;
            }

            let related_rows: Vec<&&FieldMap> = if rel.kind == ash_core::RelKind::ManyToMany {
                let through_def = match rel.through {
                    Some(f) => f(),
                    None => return Err(Error::Invalid("many_to_many requires through".into())),
                };
                let source_on_join = rel
                    .source_attribute_on_join_resource
                    .unwrap_or(rel.source_attribute);
                let dest_on_join = rel
                    .destination_attribute_on_join_resource
                    .unwrap_or(rel.destination_attribute);
                let join_rows: Vec<&FieldMap> = tables
                    .get(through_def.name)
                    .map(|t| t.values().collect())
                    .unwrap_or_default();
                let matching_dest_ids: Vec<&Value> = join_rows
                    .iter()
                    .filter(|jr| jr.get(source_on_join) == Some(&source_val))
                    .filter_map(|jr| jr.get(dest_on_join))
                    .collect();
                dest_rows
                    .iter()
                    .filter(|dest_row| {
                        let dest_val = dest_row
                            .get(rel.destination_attribute)
                            .unwrap_or(&Value::Null);
                        matching_dest_ids.contains(&dest_val)
                    })
                    .collect()
            } else {
                dest_rows
                    .iter()
                    .filter(|dest_row| {
                        let dest_val = dest_row
                            .get(rel.destination_attribute)
                            .cloned()
                            .unwrap_or(Value::Null);
                        dest_val == source_val
                    })
                    .collect()
            };

            let matching: Vec<&&FieldMap> = related_rows
                .into_iter()
                .filter(|dest_row| {
                    if let Some(filter) = &agg.filter {
                        match filter {
                            AggregateFilter::Eq(f, v) => {
                                let val = dest_row.get(*f).unwrap_or(&Value::Null);
                                !val.is_null() && val == &Value::from(*v)
                            }
                            AggregateFilter::Ne(f, v) => {
                                let val = dest_row.get(*f).unwrap_or(&Value::Null);
                                !val.is_null() && val != &Value::from(*v)
                            }
                        }
                    } else {
                        true
                    }
                })
                .collect();

            let val = match agg.kind {
                AggregateKind::Count => Value::Int(matching.len() as i64),
                AggregateKind::Exists => Value::Bool(!matching.is_empty()),
                AggregateKind::First { field } => matching
                    .first()
                    .and_then(|r| r.get(field).cloned())
                    .unwrap_or(Value::Null),
                AggregateKind::Sum { field } => {
                    let mut sum: i64 = 0;
                    for r in matching {
                        if let Some(Value::Int(n)) = r.get(field) {
                            sum += *n;
                        }
                    }
                    Value::Int(sum)
                }
            };
            row.insert(agg.name.to_string(), val);
        }
    }
    Ok(())
}

impl SchemaSupport for Memory {
    async fn install_resources(&self, _resources: &[&ResourceDef]) -> Result<()> {
        Ok(())
    }
}

impl TransactionSupport for Memory {
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let snapshot = {
            let tables = self.lock()?;
            tables.clone()
        };

        let tx_mem = Memory::from_tables(snapshot.clone());

        match f(&tx_mem).await {
            Ok(val) => {
                let committed = {
                    let tables = tx_mem.lock()?;
                    tables.clone()
                };
                let mut live = self.lock()?;

                // 1. Detect concurrent write conflicts for rows touched by tx_mem
                for (table_name, committed_table) in &committed {
                    let snap_table = snapshot.get(table_name);
                    for (id, comm_row) in committed_table {
                        let snap_row = snap_table.and_then(|t| t.get(id));
                        if snap_row != Some(comm_row) {
                            // Row was inserted or updated by tx_mem
                            let live_table = live.get(table_name);
                            let live_row = live_table.and_then(|t| t.get(id));
                            if live_row != snap_row {
                                return Err(Error::DataLayer(format!(
                                    "concurrent write conflict in {table_name} for record {id}"
                                )));
                            }
                        }
                    }
                }

                // Check for deletions made by tx_mem
                for (table_name, snap_table) in &snapshot {
                    if let Some(comm_table) = committed.get(table_name) {
                        for (id, snap_row) in snap_table {
                            if !comm_table.contains_key(id) {
                                // Row was deleted by tx_mem
                                let live_table = live.get(table_name);
                                let live_row = live_table.and_then(|t| t.get(id));
                                if live_row != Some(snap_row) {
                                    return Err(Error::DataLayer(format!(
                                        "concurrent write conflict in {table_name} for deleted record {id}"
                                    )));
                                }
                            }
                        }
                    } else {
                        let live_table = live.get(table_name);
                        if live_table != Some(snap_table) {
                            return Err(Error::DataLayer(format!(
                                "concurrent write conflict in deleted table {table_name}"
                            )));
                        }
                    }
                }

                // 2. Selectively apply mutations into live store (preserving untouched concurrent writes!)
                for (table_name, committed_table) in &committed {
                    let snap_table = snapshot.get(table_name);
                    let live_table = live.entry(table_name.clone()).or_default();
                    for (id, comm_row) in committed_table {
                        let snap_row = snap_table.and_then(|t| t.get(id));
                        if snap_row != Some(comm_row) {
                            live_table.insert(*id, comm_row.clone());
                        }
                    }
                }

                for (table_name, snap_table) in snapshot {
                    if let Some(comm_table) = committed.get(&table_name) {
                        if let Some(live_table) = live.get_mut(&table_name) {
                            for (id, _) in snap_table {
                                if !comm_table.contains_key(&id) {
                                    live_table.remove(&id);
                                }
                            }
                        }
                    } else {
                        live.remove(&table_name);
                    }
                }

                Ok(val)
            }
            Err(err) => Err(err),
        }
    }
}
