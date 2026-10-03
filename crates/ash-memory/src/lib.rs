//! HashMap-backed data layer, in the spirit of Ash.DataLayer.Ets.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::future::ready;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use ash_core::{
    all_of, any_of, apply_named_with_args, compare_typed, in_list, text_matches, AggregateFilter,
    AggregateKind, AttrType, CompiledQuery, DataLayer, Error, FieldMap, Filter, ResourceDef,
    Result, SchemaSupport, TransactionSupport, Value,
};
use uuid::Uuid;

/// The store, shared by clones. Reads run side by side and writes one at a time, as an
/// ETS table with `read_concurrency` does for the ETS data layer.
#[derive(Clone, Debug, Default)]
pub struct Memory {
    tables: Arc<RwLock<Tables>>,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            tables: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn from_tables(tables: HashMap<String, HashMap<Uuid, FieldMap>>) -> Self {
        Self {
            tables: Arc::new(RwLock::new(tables)),
        }
    }

    fn read(&self) -> Result<RwLockReadGuard<'_, Tables>> {
        self.tables
            .read()
            .map_err(|_| Error::DataLayer("memory store lock poisoned".into()))
    }

    fn write(&self) -> Result<RwLockWriteGuard<'_, Tables>> {
        self.tables
            .write()
            .map_err(|_| Error::DataLayer("memory store lock poisoned".into()))
    }
}

/// Refuses `fields` for record `id` if it clashes with another row on an identity.
///
/// An update passes the row as it was in `before`, and only identities whose fields it
/// changes are checked, as Ash checks them: an update that leaves an identity's fields
/// alone neither pays to scan for a clash nor is refused over one already stored.
fn check_identities(
    resource: &ResourceDef,
    table: &HashMap<Uuid, FieldMap>,
    id: Uuid,
    fields: &FieldMap,
    before: Option<&FieldMap>,
) -> Result<()> {
    for ident in resource.identities {
        // A `where:` predicate is SQL the in-memory store cannot evaluate, so it leaves
        // partial identities to the database rather than reject rows they do not cover.
        if ident.predicate.is_some() {
            continue;
        }
        if let Some(before) = before
            && ident.keys.iter().all(|k| fields.get(*k) == before.get(*k))
        {
            continue;
        }
        for (existing_id, row) in table.iter() {
            if *existing_id == id {
                continue;
            }
            let matches_all = ident.keys.iter().all(|k| {
                let new_val = fields.get(*k).filter(|v| !v.is_null());
                let existing_val = row.get(*k).filter(|v| !v.is_null());
                match (new_val, existing_val) {
                    (Some(a), Some(b)) => same_value(field_type(resource, k), a, b),
                    // NULLS NOT DISTINCT: two nulls collide.
                    (None, None) => !ident.nils_distinct,
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

/// The table holding `resource`'s rows for `tenant`. A context-tenant resource keeps a
/// table per tenant, as Ash's ETS data layer does, and its rows with no tenant (a global
/// resource's) in the shared table; every other resource has one table.
fn table_key(resource: &ResourceDef, tenant: Option<&str>) -> String {
    match (resource.multitenancy, tenant) {
        (Some(mt), Some(tenant)) if mt.strategy == ash_core::MultitenancyStrategy::Context => {
            format!("{}@{tenant}", resource.name)
        }
        _ => resource.name.to_string(),
    }
}

impl DataLayer for Memory {
    fn create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let table = tables.entry(table_key(resource, tenant)).or_default();
            if table.contains_key(&id) {
                return Err(Error::DataLayer(format!(
                    "duplicate id {id} in {}",
                    resource.name
                )));
            }
            check_identities(resource, table, id, &fields, None)?;
            table.insert(id, fields.clone());
            Ok(fields)
        })())
    }

    fn update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let table = tables.get_mut(&table_key(resource, tenant)).ok_or(Error::NotFound)?;
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
            let before = table.get(&id).ok_or(Error::NotFound)?;
            check_identities(resource, table, id, &current_row, Some(before))?;
            table.insert(id, current_row.clone());
            Ok(current_row)
        })())
    }

    fn upsert(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
        identity: &ash_core::IdentityDef,
        update_fields: &[String],
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        ready((|| {
            // Which rows a partial identity covers is decided by its SQL predicate, so
            // upserting on one here could overwrite a row the database would leave alone.
            if identity.predicate.is_some() {
                return Err(Error::Invalid(format!(
                    "the memory data layer cannot upsert on partial identity `{}` of {}",
                    identity.name, resource.name
                )));
            }
            let mut tables = self.write()?;
            let table = tables.entry(table_key(resource, tenant)).or_default();
            let existing_entry = table
                .iter()
                .find(|(_, row)| {
                    identity.keys.iter().all(|k| {
                        let new_val = fields.get(*k).filter(|v| !v.is_null());
                        let existing_val = row.get(*k).filter(|v| !v.is_null());
                        match (new_val, existing_val) {
                            (Some(a), Some(b)) => same_value(field_type(resource, k), a, b),
                            (None, None) => !identity.nils_distinct,
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
                check_identities(resource, table, id, &fields, None)?;
                table.insert(id, fields.clone());
                Ok(fields)
            }
        })())
    }

    fn destroy(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
    ) -> impl Future<Output = Result<()>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let table = tables.get_mut(&table_key(resource, tenant)).ok_or(Error::NotFound)?;
            table.remove(&id).ok_or(Error::NotFound)?;
            Ok(())
        })())
    }

    fn bulk_create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let table = tables.entry(table_key(resource, tenant)).or_default();
            let mut results = Vec::with_capacity(rows.len());
            for (id, fields) in rows {
                if table.contains_key(&id) {
                    return Err(Error::DataLayer(format!(
                        "duplicate id {id} in {}",
                        resource.name
                    )));
                }
                check_identities(resource, table, id, &fields, None)?;
                table.insert(id, fields.clone());
                results.push(fields);
            }
            Ok(results)
        })())
    }

    fn bulk_destroy(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        ids: &[Uuid],
    ) -> impl Future<Output = Result<()>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let table = tables.get_mut(&table_key(resource, tenant)).ok_or(Error::NotFound)?;
            for id in ids {
                table.remove(id);
            }
            Ok(())
        })())
    }

    fn can_update_atomically(&self, _resource: &ResourceDef) -> bool {
        true
    }

    /// The update as one step under the store's lock, as Ash's ETS layer runs an atomic
    /// update: the records the query selects, each checked against the conditions and
    /// set from its values as they were.
    fn update_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        update: &ash_core::AtomicUpdate,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let tenant = query.tenant.as_deref();
            let key = table_key(resource, tenant);
            let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");

            let updated = {
                let tables: &Tables = &tables;
                let mut rows: Vec<FieldMap> = tables
                    .get(&key)
                    .map(|table| table.values().cloned().collect())
                    .unwrap_or_default();
                let mut computed = Vec::new();
                let mut needed = Vec::new();
                if let Some(filter) = &query.filter {
                    filter.collect_fields(&mut needed);
                }
                compute(tables, tenant, resource, query, &mut rows, &needed, &mut computed)?;
                if let Some(filter) = &query.filter {
                    rows.retain(|row| row_matches_filter(tables, tenant, resource, filter, row));
                }
                if let Some(limit) = query.limit {
                    rows.truncate(limit);
                }
                let matches = |filter: &Filter, row: &FieldMap| row_matches_filter(tables, tenant, resource, filter, row);
                let mut updated = Vec::with_capacity(rows.len());
                for row in &rows {
                    let mut row = update.apply(resource, row, &matches)?;
                    row.retain(|name, _| resource.attribute(name).is_some());
                    updated.push(row);
                }
                updated
            };

            let table = tables.entry(key).or_default();
            for row in &updated {
                let id = row.get(pk).and_then(Value::as_uuid).ok_or(Error::NotFound)?;
                check_identities(resource, table, id, row, table.get(&id))?;
            }
            for row in &updated {
                if let Some(id) = row.get(pk).and_then(Value::as_uuid) {
                    table.insert(id, row.clone());
                }
            }
            Ok(updated)
        })())
    }

    fn can_destroy_atomically(&self, _resource: &ResourceDef) -> bool {
        true
    }

    /// The destroy as one step under the store's lock, as Ash's ETS layer runs an atomic
    /// destroy: the records the query selects, each checked against the conditions, then
    /// removed.
    fn destroy_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        conditions: &[ash_core::AtomicCondition],
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let mut tables = self.write()?;
            let tenant = query.tenant.as_deref();
            let key = table_key(resource, tenant);
            let pk = resource.primary_key().map(|attr| attr.name).unwrap_or("id");

            let destroyed = {
                let tables: &Tables = &tables;
                let mut rows: Vec<FieldMap> = tables
                    .get(&key)
                    .map(|table| table.values().cloned().collect())
                    .unwrap_or_default();
                let mut computed = Vec::new();
                let mut needed = Vec::new();
                if let Some(filter) = &query.filter {
                    filter.collect_fields(&mut needed);
                }
                compute(tables, tenant, resource, query, &mut rows, &needed, &mut computed)?;
                if let Some(filter) = &query.filter {
                    rows.retain(|row| row_matches_filter(tables, tenant, resource, filter, row));
                }
                if let Some(limit) = query.limit {
                    rows.truncate(limit);
                }
                let matches = |filter: &Filter, row: &FieldMap| row_matches_filter(tables, tenant, resource, filter, row);
                for row in &rows {
                    for condition in conditions {
                        if condition.fails_when.eval(resource, row, &matches) == Value::Bool(true) {
                            return Err((condition.error)(row));
                        }
                    }
                }
                rows
            };

            let Some(table) = tables.get_mut(&key) else {
                return Ok(Vec::new());
            };
            let mut removed = Vec::with_capacity(destroyed.len());
            for row in destroyed {
                if let Some(stored) = row.get(pk).and_then(Value::as_uuid).and_then(|id| table.remove(&id)) {
                    removed.push(stored);
                }
            }
            Ok(removed)
        })())
    }

    fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        ready((|| {
            let tables = self.read()?;
            let tenant = query.tenant.as_deref();
            let mut rows: Vec<FieldMap> = tables
                .get(&table_key(resource, tenant))
                .map(|table| table.values().cloned().collect())
                .unwrap_or_default();

            // As Ash's ETS layer does, compute calculations and aggregates for the rows
            // that survive the filter and the page, not the whole table: first only those
            // the filter needs, then those the sort needs, then the rest requested.
            let mut computed = Vec::new();
            let mut by_filter = Vec::new();
            if let Some(filter) = &query.filter {
                filter.collect_fields(&mut by_filter);
            }
            compute(&tables, tenant, resource, query, &mut rows, &by_filter, &mut computed)?;
            if let Some(filter) = &query.filter {
                rows.retain(|row| row_matches_filter(&tables, tenant, resource, filter, row));
            }

            let by_sort: Vec<&str> = query.sort.iter().map(|sort| sort.field.as_str()).collect();
            compute(&tables, tenant, resource, query, &mut rows, &by_sort, &mut computed)?;
            if !query.sort.is_empty() {
                // A guarded sort reads its field as null where the guard doesn't hold.
                let sort_value = |row: &FieldMap, sort: &ash_core::Sort| match &sort.guard {
                    Some(guard) if !row_matches_filter(&tables, tenant, resource, guard, row) => Value::Null,
                    _ => row.get(&sort.field).cloned().unwrap_or(Value::Null),
                };
                rows.sort_by(|left, right| {
                    let mut order = Ordering::Equal;
                    for sort in &query.sort {
                        let left_value = sort_value(left, sort);
                        let right_value = sort_value(right, sort);
                        // Nulls sort where the sort places them: by default last
                        // ascending and first descending, as Ash and Postgres order them.
                        let nulls_first = sort.nulls_first();
                        order = match (left_value.is_null(), right_value.is_null()) {
                            (true, true) => Ordering::Equal,
                            (true, false) if nulls_first => Ordering::Less,
                            (true, false) => Ordering::Greater,
                            (false, true) if nulls_first => Ordering::Greater,
                            (false, true) => Ordering::Less,
                            (false, false) => {
                                let order = compare_typed(field_type(resource, &sort.field), &left_value, &right_value);
                                if sort.descending { order.reverse() } else { order }
                            }
                        };
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

            let requested: Vec<&str> = query
                .calculations
                .iter()
                .chain(&query.aggregates)
                .map(String::as_str)
                .collect();
            compute(&tables, tenant, resource, query, &mut rows, &requested, &mut computed)?;

            // Only the attributes selected, as Ash's ETS layer returns them.
            for row in &mut rows {
                strip_unrequested_calculations(resource, query, row);
                strip_unrequested_aggregates(resource, query, row);
                if query.select.is_some() {
                    row.retain(|name, _| resource.attribute(name).is_none_or(|attr| query.reads(resource, attr)));
                }
            }

            Ok(rows)
        })())
    }
}

fn is_ci_string(resource: &ResourceDef, field: &str) -> bool {
    field_type(resource, field) == Some(AttrType::CiString)
}

type Tables = HashMap<String, HashMap<Uuid, FieldMap>>;

/// `tenant` is the query's: rows reached through a relationship are limited to it, as
/// the rows of the query itself are.
fn row_matches_filter(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &ResourceDef,
    filter: &Filter,
    row: &FieldMap,
) -> bool {
    eval_filter(tables, tenant, resource, filter, row, None) == Some(true)
}

/// Resources whose primary-read filters are being applied, innermost first. As in the
/// SQL compiler, a read filter that leads back to its own resource is not applied again
/// inside itself, which would recurse.
struct Applying<'a> {
    name: &'static str,
    outer: Option<&'a Applying<'a>>,
}

fn is_applying(scope: Option<&Applying>, name: &str) -> bool {
    let mut scope = scope;
    while let Some(applying) = scope {
        if applying.name == name {
            return true;
        }
        scope = applying.outer;
    }
    false
}

/// Whether `row` is in the query's tenant and passes `resource`'s primary-read filter,
/// as every read through a relationship must. The read filter is skipped when it is
/// already being applied further out.
fn passes_read_filter(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &'static ResourceDef,
    row: &FieldMap,
    scope: Option<&Applying>,
) -> bool {
    if let Some(tenant_filter) = resource.tenant_filter(tenant)
        && eval_filter(tables, tenant, resource, &tenant_filter, row, scope) != Some(true)
    {
        return false;
    }
    if is_applying(scope, resource.name) {
        return true;
    }
    let Some(read_filter) = resource.primary_read_filter() else {
        return true;
    };
    let inner = Applying {
        name: resource.name,
        outer: scope,
    };
    let row = with_calculations(resource, &read_filter, row);
    eval_filter(tables, tenant, resource, &read_filter, &row, Some(&inner)) == Some(true)
}

/// `row` with the calculations `filter` reads. Stored rows hold none, and the SQL
/// data layers compute them inline wherever a filter uses them.
fn with_calculations<'r>(
    resource: &ResourceDef,
    filter: &Filter,
    row: &'r FieldMap,
) -> std::borrow::Cow<'r, FieldMap> {
    let mut names = Vec::new();
    filter.collect_fields(&mut names);
    names.retain(|name| resource.calculation(name).is_some() && !row.contains_key(*name));
    if names.is_empty() {
        return std::borrow::Cow::Borrowed(row);
    }
    let mut row = row.clone();
    let no_args = FieldMap::new();
    for name in names {
        // A calculation that cannot run leaves its field missing, so it compares as null.
        let _ = apply_named_with_args(resource, &mut row, name, &no_args);
    }
    std::borrow::Cow::Owned(row)
}

/// Evaluates `filter` with SQL's three-valued logic: a comparison with a null field
/// is unknown (`None`), and `NOT` of unknown stays unknown, so the row is left out.
fn eval_filter(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &ResourceDef,
    filter: &Filter,
    row: &FieldMap,
    scope: Option<&Applying>,
) -> Option<bool> {
    let present = |field: &str| row.get(field).filter(|got| !got.is_null());
    let text = |field: &str, needle: &str, test: fn(&str, &str) -> bool| {
        present(field).map(|got| text_matches(Some(got), needle, is_ci_string(resource, field), test))
    };
    match filter {
        Filter::True => Some(true),
        Filter::False => Some(false),
        Filter::Eq(field, value) if value.is_null() => Some(present(field).is_none()),
        Filter::Ne(field, value) if value.is_null() => Some(present(field).is_some()),
        Filter::Eq(field, value) => {
            let ty = field_type(resource, field);
            present(field).map(|got| same_value(ty, got, value))
        }
        Filter::Ne(field, value) => {
            let ty = field_type(resource, field);
            present(field).map(|got| !same_value(ty, got, value))
        }
        Filter::Gt(field, value) => {
            compare(field_type(resource, field), row.get(field), value, Ordering::Greater, false)
        }
        Filter::Gte(field, value) => {
            compare(field_type(resource, field), row.get(field), value, Ordering::Greater, true)
        }
        Filter::Lt(field, value) => {
            compare(field_type(resource, field), row.get(field), value, Ordering::Less, false)
        }
        Filter::Lte(field, value) => {
            compare(field_type(resource, field), row.get(field), value, Ordering::Less, true)
        }
        Filter::In(_, values) if values.is_empty() => Some(false),
        Filter::In(field, values) => {
            let ty = field_type(resource, field);
            in_list(present(field), values, |got, value| same_value(ty, got, value))
        }
        Filter::IsNil(field) => Some(present(field).is_none()),
        Filter::Contains(field, needle) => text(field, needle, |text, needle| text.contains(needle)),
        Filter::StartsWith(field, needle) => {
            text(field, needle, |text, needle| text.starts_with(needle))
        }
        Filter::EndsWith(field, needle) => text(field, needle, |text, needle| text.ends_with(needle)),
        Filter::Like(field, pattern) => text(field, pattern, ash_core::like_matches),
        Filter::ILike(field, pattern) => {
            present(field).map(|got| text_matches(Some(got), pattern, true, ash_core::like_matches))
        }
        Filter::And(parts) => {
            all_of(parts.iter().map(|part| eval_filter(tables, tenant, resource, part, row, scope)))
        }
        Filter::Or(parts) => {
            any_of(parts.iter().map(|part| eval_filter(tables, tenant, resource, part, row, scope)))
        }
        Filter::Not(inner) => {
            eval_filter(tables, tenant, resource, inner, row, scope).map(|matched| !matched)
        }
        // `EXISTS (...)` in SQL: true or false, never unknown.
        Filter::Related { relationship, filter: rel_filter } => Some((|| {
            let Some(rel) = resource.relationship(relationship) else {
                return false;
            };
            let dest_res = (rel.destination)();
            let dest_table = tables.get(&table_key(dest_res, tenant));
            let Some(dest_table) = dest_table else {
                return false;
            };
            let matches = |dest_row: &FieldMap| {
                let dest_row = with_calculations(dest_res, rel_filter, dest_row);
                eval_filter(tables, tenant, dest_res, rel_filter, &dest_row, scope) == Some(true)
                    && passes_read_filter(tables, tenant, dest_res, &dest_row, scope)
            };

            match rel.kind {
                ash_core::RelKind::BelongsTo
                | ash_core::RelKind::HasMany
                | ash_core::RelKind::HasOne => {
                    let Some(key) = rel.source_key(row) else {
                        return false;
                    };
                    dest_table.values().any(|dest_row| {
                        rel.destination_key(dest_row).as_ref() == Some(&key) && matches(dest_row)
                    })
                }
                ash_core::RelKind::ManyToMany => {
                    let Some(through_fn) = rel.through else {
                        return false;
                    };
                    let through_res = through_fn();
                    let through_table = tables.get(&table_key(through_res, tenant));
                    let Some(through_table) = through_table else {
                        return false;
                    };
                    let Some(source_val) = row.get(rel.source_attribute) else {
                        return false;
                    };
                    let source_on_join = rel.source_attribute_on_join_resource.unwrap_or(rel.source_attribute);
                    let dest_on_join = rel.destination_attribute_on_join_resource.unwrap_or(rel.destination_attribute);

                    // An archived join row unlinks the records, as it does for loads.
                    let matching_dest_ids: Vec<&Value> = through_table
                        .values()
                        .filter(|jr| jr.get(source_on_join) == Some(source_val))
                        .filter(|jr| passes_read_filter(tables, tenant, through_res, jr, scope))
                        .filter_map(|jr| jr.get(dest_on_join))
                        .collect();

                    dest_table.values().any(|dest_row| {
                        let dest_id = dest_row.get(rel.destination_attribute).unwrap_or(&Value::Null);
                        matching_dest_ids.contains(&dest_id) && matches(dest_row)
                    })
                }
            }
        })()),
    }
}

/// The attribute or calculation type of `field`, which decides how values compare.
fn field_type(resource: &ResourceDef, field: &str) -> Option<AttrType> {
    resource
        .attribute(field)
        .map(|attr| attr.ty)
        .or_else(|| resource.calculation(field).map(|calc| calc.ty))
}

fn same_value(ty: Option<AttrType>, a: &Value, b: &Value) -> bool {
    compare_typed(ty, a, b) == Ordering::Equal
}

fn compare(
    ty: Option<AttrType>,
    got: Option<&Value>,
    rhs: &Value,
    direction: Ordering,
    equal_ok: bool,
) -> Option<bool> {
    let got = got.filter(|got| !got.is_null())?;
    if rhs.is_null() {
        return None;
    }
    Some(match compare_typed(ty, got, rhs) {
        Ordering::Equal => equal_ok,
        order => order == direction,
    })
}

/// Computes, on every row, the calculations and then the aggregates among `names` that
/// aren't in `computed` yet, and records them there.
fn compute<'a>(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &ResourceDef,
    query: &CompiledQuery,
    rows: &mut [FieldMap],
    names: &[&'a str],
    computed: &mut Vec<&'a str>,
) -> Result<()> {
    let fresh = |computed: &[&str], name: &&'a str| !computed.contains(name);
    let mut calculations: Vec<&'a str> = names
        .iter()
        .filter(|name| resource.calculation(name).is_some() && fresh(computed, name))
        .copied()
        .collect();
    calculations.sort_unstable();
    calculations.dedup();
    let empty_args = FieldMap::new();
    for row in rows.iter_mut() {
        for name in &calculations {
            let args = query.calculation_args.get(*name).unwrap_or(&empty_args);
            apply_named_with_args(resource, row, name, args)?;
        }
    }
    computed.extend(&calculations);

    let mut aggregates: Vec<&'a str> = names
        .iter()
        .filter(|name| resource.aggregate(name).is_some() && fresh(computed, name))
        .copied()
        .collect();
    aggregates.sort_unstable();
    aggregates.dedup();
    if !aggregates.is_empty() {
        apply_aggregates(tables, tenant, resource, rows, &aggregates, query.actor.as_ref())?;
    }
    computed.extend(&aggregates);
    Ok(())
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

/// Whether `actor` may read `row` of `resource`, as its primary read's policies say, as
/// Ash authorizes an aggregate's query by default.
fn actor_reads(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &'static ResourceDef,
    policy: &std::result::Result<Option<Filter>, ()>,
    row: &FieldMap,
) -> bool {
    match policy {
        Ok(None) => true,
        Ok(Some(filter)) => {
            // A calculation the policy checks that fails to compute denies the row, rather
            // than reading as nil.
            let mut names = Vec::new();
            filter.collect_fields(&mut names);
            let mut row = row.clone();
            for name in names.into_iter().filter(|name| resource.calculation(name).is_some()) {
                if !row.contains_key(name) && apply_named_with_args(resource, &mut row, name, &FieldMap::new()).is_err() {
                    return false;
                }
            }
            eval_filter(tables, tenant, resource, filter, &row, None) == Some(true)
        }
        Err(()) => false,
    }
}

/// `resource`'s read policies for `actor`: a filter, none, or `Err` if it may read nothing.
fn read_policy(resource: &'static ResourceDef, actor: Option<&ash_core::Actor>) -> Result<std::result::Result<Option<Filter>, ()>> {
    match ash_core::compile_read_filter(resource, resource.default_read(), actor) {
        Ok(filter) => Ok(Ok(filter)),
        Err(Error::Forbidden) => Ok(Err(())),
        Err(err) => Err(err),
    }
}

fn apply_aggregates(
    tables: &Tables,
    tenant: Option<&str>,
    resource: &ResourceDef,
    rows: &mut [FieldMap],
    needed: &[&str],
    actor: Option<&ash_core::Actor>,
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
        let dest_policy = read_policy(dest, actor)?;
        let dest_rows: Vec<&FieldMap> = tables
            .get(&table_key(dest, tenant))
            .map(|t| {
                t.values()
                    .filter(|dest_row| passes_read_filter(tables, tenant, dest, dest_row, None))
                    .filter(|dest_row| actor_reads(tables, tenant, dest, &dest_policy, dest_row))
                    .collect()
            })
            .unwrap_or_default();

        for row in rows.iter_mut() {
            let source_val = row.get(rel.source_attribute).cloned().unwrap_or(Value::Null);
            if source_val.is_null() {
                let default_val = match agg.kind {
                    AggregateKind::Count => Value::Int(0),
                    AggregateKind::Exists => Value::Bool(false),
                    AggregateKind::First { .. } => Value::Null,
                    // A sum of nothing is nil, as Ash's is.
                    AggregateKind::Sum { .. } => Value::Null,
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
                let through_policy = read_policy(through_def, actor)?;
                let join_rows: Vec<&FieldMap> = tables
                    .get(&table_key(through_def, tenant))
                    .map(|t| {
                        t.values()
                            .filter(|jr| passes_read_filter(tables, tenant, through_def, jr, None))
                            .filter(|jr| actor_reads(tables, tenant, through_def, &through_policy, jr))
                            .collect()
                    })
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
                let key = rel.source_key(row);
                dest_rows
                    .iter()
                    .filter(|dest_row| key.is_some() && rel.destination_key(dest_row) == key)
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
                // As SQL's SUM: nil values are skipped, and a sum of none is nil.
                AggregateKind::Sum { field } => matching
                    .iter()
                    .filter_map(|r| r.get(field).and_then(Value::as_int))
                    .reduce(|sum, n| sum + n)
                    .map_or(Value::Null, Value::Int),
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
            let tables = self.read()?;
            tables.clone()
        };

        let tx_mem = Memory::from_tables(snapshot.clone());

        match f(&tx_mem).await {
            Ok(val) => {
                let committed = {
                    let tables = tx_mem.read()?;
                    tables.clone()
                };
                let mut live = self.write()?;

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
