use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;

use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, ManagedRelType, PersistKind};
use crate::changeset::{self, Changeset, ManagedRelationshipSpec};
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::keys::{AggregateName, CalcName, FieldName, RelName};
use crate::pipeline::{
    action_named, expect_kind, expect_persist, generate_pk, pk_name, read_action, run_validations,
    validate,
};
use crate::policy::{authorize_write, compile_read_filter};
use crate::resource::{OnDelete, RelKind, Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

pub struct Query<'a, R, D> {
    ctx: &'a Context<D>,
    action: Option<&'static str>,
    filter: Option<Filter>,
    sort: Vec<Sort>,
    calculations: Vec<String>,
    aggregates: Vec<String>,
    loads: Vec<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    _resource: PhantomData<R>,
}

impl<'a, R, D> Clone for Query<'a, R, D> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx,
            action: self.action,
            filter: self.filter.clone(),
            sort: self.sort.clone(),
            calculations: self.calculations.clone(),
            aggregates: self.aggregates.clone(),
            loads: self.loads.clone(),
            limit: self.limit,
            offset: self.offset,
            _resource: PhantomData,
        }
    }
}

impl<'a, R: Resource, D: DataLayer> Query<'a, R, D> {
    pub fn action(mut self, name: &'static str) -> Self {
        self.action = Some(name);
        self
    }

    pub fn filter(mut self, filter: Filter) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(existing) => Filter::and([existing, filter]),
            None => filter,
        });
        self
    }

    pub fn sort(self, field: impl FieldName<R>) -> Self {
        self.sort_by(field, false)
    }

    pub fn sort_desc(self, field: impl FieldName<R>) -> Self {
        self.sort_by(field, true)
    }

    pub fn sort_by(mut self, field: impl FieldName<R>, descending: bool) -> Self {
        self.sort.push(Sort {
            field: field.as_field().to_string(),
            descending,
        });
        self
    }

    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }

    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    pub fn calc(mut self, name: impl CalcName<R>) -> Self {
        self.calculations.push(name.as_calc().to_string());
        self
    }

    pub fn aggregate(mut self, name: impl AggregateName<R>) -> Self {
        self.aggregates.push(name.as_aggregate().to_string());
        self
    }

    pub fn load_aggregate(self, name: impl AggregateName<R>) -> Self {
        self.aggregate(name)
    }

    pub fn load_rel(mut self, name: impl RelName<R>) -> Self {
        self.loads.push(name.as_rel().to_string());
        self
    }

    pub fn include(self, name: impl RelName<R>) -> Self {
        self.load_rel(name)
    }

    fn apply_preparations(mut self, action: &crate::action::ActionDef) -> Self {
        for prep in action.preparations {
            match *prep {
                crate::action::PreparationDef::Filter(filter_fn) => {
                    let prep_filter = filter_fn();
                    self.filter = match self.filter {
                        Some(user_filter) => Some(Filter::and([prep_filter, user_filter])),
                        None => Some(prep_filter),
                    };
                }
                crate::action::PreparationDef::Sort { field, descending } => {
                    if self.sort.is_empty() {
                        self.sort.push(Sort {
                            field: field.to_string(),
                            descending,
                        });
                    }
                }
                crate::action::PreparationDef::Limit(limit) => {
                    if self.limit.is_none() {
                        self.limit = Some(limit);
                    }
                }
                crate::action::PreparationDef::Offset(offset) => {
                    if self.offset.is_none() {
                        self.offset = Some(offset);
                    }
                }
            }
        }
        self
    }

    pub async fn load(self) -> Result<Vec<R>> {
        let action = read_action(&R::DEF, self.action)?;
        let this = self.apply_preparations(action);
        let policy_filter = compile_read_filter(&R::DEF, action, this.ctx.actor.as_ref())?;
        let filter = match (this.filter, policy_filter) {
            (None, None) => None,
            (Some(filter), None) | (None, Some(filter)) => Some(filter),
            (Some(user), Some(policy)) => Some(Filter::and([user, policy])),
        };

        for name in &this.calculations {
            if R::DEF.calculation(name).is_none() {
                return Err(Error::Invalid(format!(
                    "unknown calculation `{name}` on {}",
                    R::DEF.name
                )));
            }
        }

        for name in &this.aggregates {
            if R::DEF.aggregate(name).is_none() {
                return Err(Error::Invalid(format!(
                    "unknown aggregate `{name}` on {}",
                    R::DEF.name
                )));
            }
        }

        let rows = this
            .ctx
            .data
            .run_query(
                &R::DEF,
                &CompiledQuery {
                    filter,
                    sort: this.sort,
                    calculations: this.calculations,
                    aggregates: this.aggregates,
                    limit: this.limit,
                    offset: this.offset,
                },
            )
            .await?;

        let mut records = Vec::with_capacity(rows.len());
        for mut row in rows {
            crate::policy::redact_fields(&R::DEF, this.ctx.actor.as_ref(), &mut row)?;
            records.push(R::from_fields(&row)?);
        }
        attach_relationships(this.ctx, &mut records, &this.loads).await?;
        Ok(records)
    }

    pub async fn all(self) -> Result<Vec<R>> {
        self.load().await
    }

    pub async fn first(self) -> Result<Option<R>> {
        let mut rows = self.limit(1).load().await?;
        Ok(rows.pop())
    }

    pub async fn one(self) -> Result<R> {
        let mut rows = self.limit(2).load().await?;
        match rows.len() {
            0 => Err(Error::NotFound),
            1 => Ok(rows.remove(0)),
            n => Err(Error::TooMany(n)),
        }
    }

    pub async fn count(self) -> Result<usize> {
        let action = read_action(&R::DEF, self.action)?;
        let this = self.apply_preparations(action);
        let policy_filter = compile_read_filter(&R::DEF, action, this.ctx.actor.as_ref())?;
        let filter = match (this.filter, policy_filter) {
            (None, None) => None,
            (Some(filter), None) | (None, Some(filter)) => Some(filter),
            (Some(user), Some(policy)) => Some(Filter::and([user, policy])),
        };

        let rows = this
            .ctx
            .data
            .run_query(
                &R::DEF,
                &CompiledQuery {
                    filter,
                    sort: Vec::new(),
                    calculations: Vec::new(),
                    aggregates: Vec::new(),
                    limit: None,
                    offset: None,
                },
            )
            .await?;
        Ok(rows.len())
    }

    pub async fn page_offset(
        mut self,
        limit: usize,
        offset: usize,
        count_total: bool,
    ) -> Result<Page<R>> {
        let total_count = if count_total {
            let count_query = Query {
                ctx: self.ctx,
                action: self.action,
                filter: self.filter.clone(),
                sort: Vec::new(),
                calculations: Vec::new(),
                aggregates: Vec::new(),
                loads: Vec::new(),
                limit: None,
                offset: None,
                _resource: PhantomData::<R>,
            };
            Some(count_query.count().await?)
        } else {
            None
        };

        self.limit = Some(limit + 1);
        self.offset = Some(offset);
        let mut results = self.load().await?;
        let has_more = results.len() > limit;
        if has_more {
            results.truncate(limit);
        }

        let after = results.last().map(|r| r.id().to_string());
        let before = results.first().map(|r| r.id().to_string());

        Ok(Page {
            results,
            has_more,
            limit,
            offset: Some(offset),
            total_count,
            after,
            before,
        })
    }

    pub async fn page_keyset(
        mut self,
        limit: usize,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<Page<R>> {
        let pk = pk_name(&R::DEF)?.to_string();

        if self.sort.is_empty() {
            self.sort.push(Sort {
                field: pk.clone(),
                descending: false,
            });
        } else if !self.sort.iter().any(|s| s.field == pk) {
            let last_desc = self.sort.last().map(|s| s.descending).unwrap_or(false);
            self.sort.push(Sort {
                field: pk.clone(),
                descending: last_desc,
            });
        }

        let is_before = before.is_some() && after.is_none();
        let target_cursor_str = if is_before { before } else { after };

        if let Some(c_str) = target_cursor_str
            && let Some(mut cursor) = KeysetCursor::decode(c_str)
        {
            if cursor.values.is_empty() {
                    if let Ok(rec) = get::<R, D>(self.ctx, cursor.id).await {
                        let fields = R::to_fields(&rec);
                        for s in &self.sort {
                            let val = fields.get(&s.field).cloned().unwrap_or(Value::Null);
                            cursor.values.push((s.field.clone(), val));
                        }
                    } else {
                        cursor.values.push((pk.clone(), Value::Uuid(cursor.id)));
                    }
                }

                let mut sort_tuples = Vec::new();
                for s in &self.sort {
                    let val = cursor
                        .values
                        .iter()
                        .find(|(k, _)| k == &s.field)
                        .map(|(_, v)| v.clone())
                        .unwrap_or(Value::Null);
                    sort_tuples.push((s.field.clone(), val, s.descending));
                }

                if let Some(cursor_filter) = build_keyset_filter(&sort_tuples, !is_before) {
                    self = self.filter(cursor_filter);
                }
            }

        let effective_sort = self.sort.clone();

        if is_before {
            for s in &mut self.sort {
                s.descending = !s.descending;
            }
        }

        self.limit = Some(limit + 1);
        let mut results = self.load().await?;
        let has_more = results.len() > limit;
        if has_more {
            results.truncate(limit);
        }

        if is_before {
            results.reverse();
        }

        let after_cursor = results.last().map(|r| cursor_for_record(r, &effective_sort));
        let before_cursor = results.first().map(|r| cursor_for_record(r, &effective_sort));

        Ok(Page {
            results,
            has_more,
            limit,
            offset: None,
            total_count: None,
            after: after_cursor,
            before: before_cursor,
        })
    }

    /// Bulk destroy all records matching this query.
    pub async fn bulk_destroy(
        self,
        action: &str,
        opts: crate::bulk::BulkDestroyOptions,
    ) -> Result<crate::bulk::BulkResult<R>> {
        let records = self.clone().load().await?;
        let ids: Vec<Uuid> = records.iter().map(|r| r.id()).collect();
        crate::bulk::bulk_destroy::<R, D>(self.ctx, action, &ids, opts).await
    }

    /// Process query results in chunks of `chunk_size` without loading all records into memory at once.
    ///
    /// The handler closure is called sequentially for each chunk. If the handler returns an error,
    /// iteration halts and the error is returned.
    ///
    /// Returns the total number of records processed.
    pub async fn chunked<F, Fut>(
        self,
        chunk_size: usize,
        mut handler: F,
    ) -> Result<usize>
    where
        F: FnMut(Vec<R>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        if chunk_size == 0 {
            return Err(Error::Invalid("chunk_size must be greater than 0".into()));
        }
        let mut total = 0;
        let mut current_offset = self.offset.unwrap_or(0);
        let original_limit = self.limit;

        loop {
            let this_chunk_size = match original_limit {
                Some(max_limit) => {
                    let remaining = max_limit.saturating_sub(total);
                    if remaining == 0 {
                        break;
                    }
                    chunk_size.min(remaining)
                }
                None => chunk_size,
            };

            let mut chunk_query = self.clone();
            chunk_query.limit = Some(this_chunk_size);
            chunk_query.offset = Some(current_offset);

            let rows = chunk_query.load().await?;
            let count = rows.len();
            if count == 0 {
                break;
            }

            total += count;
            handler(rows).await?;

            if count < this_chunk_size {
                break;
            }
            current_offset += count;
        }

        Ok(total)
    }

    /// Keyset-based chunking for high-performance iteration over ordered large datasets.
    pub async fn chunked_keyset<F, Fut>(
        self,
        chunk_size: usize,
        mut handler: F,
    ) -> Result<usize>
    where
        F: FnMut(Vec<R>) -> Fut,
        Fut: Future<Output = Result<()>>,
    {
        if chunk_size == 0 {
            return Err(Error::Invalid("chunk_size must be greater than 0".into()));
        }
        let mut total = 0;
        let mut after_cursor: Option<String> = None;

        loop {
            let page = self
                .clone()
                .page_keyset(chunk_size, after_cursor.as_deref(), None)
                .await?;

            let count = page.results.len();
            if count == 0 {
                break;
            }

            total += count;
            after_cursor = page.after;
            let has_more = page.has_more;

            handler(page.results).await?;

            if !has_more || after_cursor.is_none() {
                break;
            }
        }

        Ok(total)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct KeysetCursor {
    pub id: Uuid,
    pub values: Vec<(String, Value)>,
}

impl KeysetCursor {
    pub fn encode(&self) -> String {
        use base64::prelude::*;
        let json = serde_json::to_string(self).unwrap_or_default();
        BASE64_URL_SAFE_NO_PAD.encode(json.as_bytes())
    }

    pub fn decode(cursor_str: &str) -> Option<Self> {
        use base64::prelude::*;
        if let Ok(bytes) = BASE64_URL_SAFE_NO_PAD.decode(cursor_str.as_bytes())
            && let Ok(cursor) = serde_json::from_slice::<Self>(&bytes)
        {
            return Some(cursor);
        }
        if let Ok(cursor) = serde_json::from_str::<Self>(cursor_str) {
            return Some(cursor);
        }
        if let Ok(id) = Uuid::parse_str(cursor_str) {
            return Some(Self {
                id,
                values: Vec::new(),
            });
        }
        None
    }
}

fn build_keyset_filter(sorts: &[(String, Value, bool)], is_after: bool) -> Option<Filter> {
    if sorts.is_empty() {
        return None;
    }
    let (field, val, desc) = &sorts[0];
    let cond = if is_after {
        if !desc {
            Filter::Gt(field.clone(), val.clone())
        } else {
            Filter::Lt(field.clone(), val.clone())
        }
    } else {
        if !desc {
            Filter::Lt(field.clone(), val.clone())
        } else {
            Filter::Gt(field.clone(), val.clone())
        }
    };

    if sorts.len() == 1 {
        Some(cond)
    } else {
        let eq = Filter::Eq(field.clone(), val.clone());
        let rest = build_keyset_filter(&sorts[1..], is_after)?;
        Some(Filter::or([cond, Filter::and([eq, rest])]))
    }
}

fn cursor_for_record<R: Resource>(record: &R, sort: &[Sort]) -> String {
    let fields = R::to_fields(record);
    let mut values = Vec::with_capacity(sort.len());
    for s in sort {
        let val = fields.get(&s.field).cloned().unwrap_or(Value::Null);
        values.push((s.field.clone(), val));
    }
    KeysetCursor {
        id: record.id(),
        values,
    }
    .encode()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub results: Vec<T>,
    pub has_more: bool,
    pub limit: usize,
    pub offset: Option<usize>,
    pub total_count: Option<usize>,
    pub after: Option<String>,
    pub before: Option<String>,
}

impl<T> Page<T> {
    pub fn results(&self) -> &[T] {
        &self.results
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub fn total_count(&self) -> Option<usize> {
        self.total_count
    }

    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }
}

pub fn query<R: Resource, D: DataLayer>(ctx: &Context<D>) -> Query<'_, R, D> {
    Query {
        ctx,
        action: None,
        filter: None,
        sort: Vec::new(),
        calculations: Vec::new(),
        aggregates: Vec::new(),
        loads: Vec::new(),
        limit: None,
        offset: None,
        _resource: PhantomData,
    }
}

async fn attach_relationships<R: Resource, D: DataLayer>(
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
            RelKind::BelongsTo => {
                let mut ids = HashSet::new();
                for record in records.iter() {
                    if let Some(Value::Uuid(id)) = record.to_fields().get(rel.source_attribute) {
                        ids.insert(*id);
                    }
                }
                let related = fetch_related(ctx, dest, rel.destination_attribute, &ids).await?;
                let by_id: HashMap<Uuid, FieldMap> = related
                    .into_iter()
                    .filter_map(|row| {
                        required_uuid(&row, rel.destination_attribute)
                            .ok()
                            .map(|id| (id, row))
                    })
                    .collect();
                for record in records.iter_mut() {
                    let attached = match record.to_fields().get(rel.source_attribute) {
                        Some(Value::Uuid(id)) => by_id.get(id).cloned().into_iter().collect(),
                        _ => Vec::new(),
                    };
                    record.attach(name, attached)?;
                }
            }
            RelKind::HasMany => {
                let ids: HashSet<Uuid> = records.iter().map(Resource::id).collect();
                let related = fetch_related(ctx, dest, rel.destination_attribute, &ids).await?;
                let mut groups: HashMap<Uuid, Vec<FieldMap>> = HashMap::new();
                for row in related {
                    if let Ok(fk) = required_uuid(&row, rel.destination_attribute) {
                        groups.entry(fk).or_default().push(row);
                    }
                }
                for record in records.iter_mut() {
                    let attached = groups.remove(&record.id()).unwrap_or_default();
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

async fn fetch_related<D: DataLayer>(
    ctx: &Context<D>,
    dest: &ResourceDef,
    id_field: &str,
    ids: &HashSet<Uuid>,
) -> Result<Vec<FieldMap>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let values: Vec<Value> = ids.iter().copied().map(Value::Uuid).collect();
    let id_filter = Filter::In(id_field.to_string(), values);
    let read = dest.primary_read().ok_or(Error::NoPrimaryRead(dest.name))?;
    let policy_filter = compile_read_filter(dest, read, ctx.actor.as_ref())?;
    let filter = match policy_filter {
        Some(policy) => Filter::and([id_filter, policy]),
        None => id_filter,
    };
    let mut rows = ctx
        .data
        .run_query(
            dest,
            &CompiledQuery {
                filter: Some(filter),
                ..CompiledQuery::default()
            },
        )
        .await?;
    for row in &mut rows {
        crate::policy::redact_fields(dest, ctx.actor.as_ref(), row)?;
    }
    Ok(rows)
}

pub async fn get<R: Resource, D: DataLayer>(ctx: &Context<D>, id: Uuid) -> Result<R> {
    let pk = pk_name(&R::DEF)?;
    query::<R, D>(ctx).filter(Filter::eq(pk, id)).one().await
}

pub async fn create<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    input: FieldMap,
) -> Result<R> {
    Changeset::<R>::for_create(ctx, action, input)?
        .commit(ctx)
        .await
}

pub async fn update<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    id: Uuid,
    input: FieldMap,
) -> Result<R> {
    let existing = get::<R, D>(ctx, id).await?;
    Changeset::for_update_on(ctx, action, existing, input)?
        .commit(ctx)
        .await
}

pub async fn update_existing<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    existing: R,
    input: FieldMap,
) -> Result<R> {
    Changeset::for_update_on(ctx, action, existing, input)?
        .commit(ctx)
        .await
}

pub async fn destroy<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    id: Uuid,
) -> Result<()> {
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Destroy)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    let existing = get::<R, D>(ctx, id).await?;
    let existing_fields = existing.to_fields();
    destroy_dynamic(ctx, &R::DEF, action_def, id, &existing_fields).await
}

pub async fn destroy_existing<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    existing: R,
) -> Result<()> {
    let cs = Changeset::for_destroy(ctx, action, existing)?;
    cs.commit(ctx).await?;
    Ok(())
}

pub async fn destroy_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    existing_fields: &FieldMap,
) -> Result<()> {
    let mut fields = existing_fields.clone();
    let mut dynamic_before_actions = Vec::new();
    let mut dynamic_after_actions = Vec::new();
    let mut dynamic_after_transactions = Vec::new();

    crate::pipeline::apply_changes_with_hooks(
        &mut fields,
        action,
        ctx.actor.as_ref(),
        &crate::value::FieldMap::new(),
        &mut dynamic_before_actions,
        &mut dynamic_after_actions,
        &mut dynamic_after_transactions,
    )?;

    for hook in dynamic_before_actions {
        hook(&mut fields)?;
    }

    authorize_write(
        resource,
        action,
        ctx.actor.as_ref(),
        Some(&fields),
    )?;

    handle_cascading_deletes(ctx, resource, id, &fields).await?;

    match ctx.data.destroy(resource, id).await {
        Ok(()) => {
            for hook in dynamic_after_actions {
                hook(&mut fields)?;
            }

            let notification = crate::notifier::Notification::new(
                resource.name,
                action.name,
                ActionKind::Destroy,
                id,
                fields.clone(),
                Some(fields.clone()),
                ctx.actor.clone(),
                crate::value::FieldMap::new(),
            );
            crate::notifier::dispatch_notification(ctx, resource, notification).await?;

            for hook in dynamic_after_transactions {
                hook(Ok(&fields));
            }
            Ok(())
        }
        Err(err) => {
            for hook in dynamic_after_transactions {
                hook(Err(&err));
            }
            Err(err)
        }
    }
}

pub async fn handle_cascading_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    parent_fields: &FieldMap,
) -> Result<()> {
    for rel in resource.relationships {
        match rel.kind {
            RelKind::HasMany => {
                match rel.on_delete {
                    OnDelete::Nothing => {}
                    OnDelete::Restrict => {
                        let dest_def = (rel.destination)();
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
                        if !rows.is_empty() {
                            return Err(Error::DeleteRestricted {
                                resource: resource.name,
                                relationship: rel.name,
                                count: rows.len(),
                            });
                        }
                    }
                    OnDelete::Cascade => {
                        let dest_def = (rel.destination)();
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
                        let child_pk = pk_name(dest_def)?;
                        let child_destroy_action = dest_def
                            .actions
                            .iter()
                            .find(|a| a.kind == ActionKind::Destroy && a.primary)
                            .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                        for child_row in rows {
                            let child_id = required_uuid(&child_row, child_pk)?;
                            if let Some(destroy_act) = child_destroy_action {
                                Box::pin(destroy_dynamic(
                                    ctx,
                                    dest_def,
                                    destroy_act,
                                    child_id,
                                    &child_row,
                                ))
                                .await?;
                            } else {
                                ctx.data.destroy(dest_def, child_id).await?;
                            }
                        }
                    }
                    OnDelete::Nilify => {
                        let dest_def = (rel.destination)();
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
                        let child_pk = pk_name(dest_def)?;
                        for child_row in rows {
                            let child_id = required_uuid(&child_row, child_pk)?;
                            let mut patch = FieldMap::new();
                            patch.insert(rel.destination_attribute.to_string(), Value::Null);
                            ctx.data.update(dest_def, child_id, patch).await?;
                        }
                    }
                }
            }
            RelKind::ManyToMany => {
                if let Some(through_fn) = rel.through {
                    let through_def = through_fn();
                    let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                    let parent_val = parent_fields
                        .get(rel.source_attribute)
                        .cloned()
                        .unwrap_or_else(|| Value::from(parent_id));
                    let filter = Filter::eq(source_fk, parent_val);
                    match rel.on_delete {
                        OnDelete::Nothing => {}
                        OnDelete::Restrict => {
                            let rows = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(filter),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;
                            if !rows.is_empty() {
                                return Err(Error::DeleteRestricted {
                                    resource: resource.name,
                                    relationship: rel.name,
                                    count: rows.len(),
                                });
                            }
                        }
                        OnDelete::Cascade => {
                            let rows = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(filter),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;
                            let join_pk = pk_name(through_def)?;
                            let join_destroy_action = through_def
                                .actions
                                .iter()
                                .find(|a| a.kind == ActionKind::Destroy && a.primary)
                                .or_else(|| through_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                            for join_row in rows {
                                let join_id = required_uuid(&join_row, join_pk)?;
                                if let Some(destroy_act) = join_destroy_action {
                                    Box::pin(destroy_dynamic(
                                        ctx,
                                        through_def,
                                        destroy_act,
                                        join_id,
                                        &join_row,
                                    ))
                                    .await?;
                                } else {
                                    ctx.data.destroy(through_def, join_id).await?;
                                }
                            }
                        }
                        OnDelete::Nilify => {}
                    }
                }
            }
            RelKind::BelongsTo => {}
        }
    }
    Ok(())
}

pub async fn create_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    mut fields: FieldMap,
) -> Result<FieldMap> {
    generate_pk(resource, &mut fields);
    let id = required_uuid(&fields, pk_name(resource)?)?;

    if let Some((created_at, updated_at)) = resource.timestamps {
        let now = crate::resource::utc_now_iso8601();
        fields.entry(created_at.to_string()).or_insert_with(|| Value::String(now.clone()));
        fields.entry(updated_at.to_string()).or_insert_with(|| Value::String(now));
    }

    for attr in resource.attributes {
        if let Some(def_fn) = attr.default_fn {
            fields.entry(attr.name.to_string()).or_insert_with(def_fn);
        }
    }

    validate(resource, &fields)?;
    run_validations(resource, action, None, &fields, &FieldMap::new())?;
    authorize_write(resource, action, ctx.actor.as_ref(), Some(&fields))?;

    let stored = ctx.data.create(resource, id, fields).await?;

    let notification = crate::notifier::Notification::new(
        resource.name,
        action.name,
        ActionKind::Create,
        id,
        stored.clone(),
        None,
        ctx.actor.clone(),
        FieldMap::new(),
    );
    crate::notifier::dispatch_notification(ctx, resource, notification).await?;

    Ok(stored)
}

pub async fn update_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    input: FieldMap,
) -> Result<FieldMap> {
    let pk = pk_name(resource)?;
    let existing_fields = ctx
        .data
        .run_query(
            resource,
            &CompiledQuery {
                filter: Some(Filter::eq(pk, Value::Uuid(id))),
                ..CompiledQuery::default()
            },
        )
        .await?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)?;

    let mut fields = existing_fields.clone();
    fields.extend(input);

    if let Some(v_attr) = resource.optimistic_lock_attribute() {
        let current_v = existing_fields
            .get(v_attr)
            .and_then(|v| match v {
                Value::Int(n) => Some(*n),
                _ => None,
            })
            .unwrap_or(1);
        fields.insert(v_attr.to_string(), Value::Int(current_v + 1));
    }

    if let Some((_created_at, updated_at)) = resource.timestamps {
        let now = crate::resource::utc_now_iso8601();
        fields.insert(updated_at.to_string(), Value::String(now));
    }

    validate(resource, &fields)?;
    run_validations(resource, action, Some(&existing_fields), &fields, &FieldMap::new())?;
    authorize_write(resource, action, ctx.actor.as_ref(), Some(&fields))?;

    let stored = ctx.data.update(resource, id, fields).await?;

    let notification = crate::notifier::Notification::new(
        resource.name,
        action.name,
        ActionKind::Update,
        id,
        stored.clone(),
        Some(existing_fields),
        ctx.actor.clone(),
        FieldMap::new(),
    );
    crate::notifier::dispatch_notification(ctx, resource, notification).await?;

    Ok(stored)
}

pub async fn handle_managed_relationships<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    managed_list: Vec<ManagedRelationshipSpec>,
) -> Result<()> {
    for managed in managed_list {
        let rel = resource
            .relationship(managed.relationship)
            .ok_or_else(|| Error::Invalid(format!("unknown relationship `{}` on {}", managed.relationship, resource.name)))?;

        match rel.kind {
            RelKind::HasMany => {
                let dest_def = (rel.destination)();
                let child_fk = rel.destination_attribute;
                let child_pk = pk_name(dest_def)?;
                let child_create_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Create && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                let child_update_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Update && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Update));
                let child_destroy_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Destroy && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                match managed.rel_type {
                    ManagedRelType::Create | ManagedRelType::Append => {
                        for mut child_fields in managed.inputs {
                            child_fields.insert(child_fk.to_string(), Value::from(parent_id));
                            if let Some(create_act) = child_create_action {
                                Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                            } else {
                                generate_pk(dest_def, &mut child_fields);
                                let child_id = required_uuid(&child_fields, child_pk)?;
                                ctx.data.create(dest_def, child_id, child_fields).await?;
                            }
                        }
                    }
                    ManagedRelType::DirectControl => {
                        let filter = Filter::eq(child_fk, Value::from(parent_id));
                        let existing_rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;

                        let mut existing_map = HashMap::new();
                        for row in existing_rows {
                            if let Ok(id) = required_uuid(&row, child_pk) {
                                existing_map.insert(id, row);
                            }
                        }

                        let mut kept_ids = HashSet::new();
                        for mut child_fields in managed.inputs {
                            child_fields.insert(child_fk.to_string(), Value::from(parent_id));
                            let given_id = child_fields
                                .get(child_pk)
                                .and_then(|v| match v {
                                    Value::Uuid(u) => Some(*u),
                                    Value::String(s) => Uuid::parse_str(s).ok(),
                                    _ => None,
                                });

                            if let Some(child_id) = given_id && existing_map.contains_key(&child_id) {
                                kept_ids.insert(child_id);
                                if let Some(update_act) = child_update_action {
                                    Box::pin(update_dynamic(ctx, dest_def, update_act, child_id, child_fields)).await?;
                                } else {
                                    ctx.data.update(dest_def, child_id, child_fields).await?;
                                }
                            } else {
                                let new_id = if let Some(create_act) = child_create_action {
                                    let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                    required_uuid(&stored, child_pk)?
                                } else {
                                    generate_pk(dest_def, &mut child_fields);
                                    let child_id = required_uuid(&child_fields, child_pk)?;
                                    ctx.data.create(dest_def, child_id, child_fields).await?;
                                    child_id
                                };
                                kept_ids.insert(new_id);
                            }
                        }

                        // Remove omitted children
                        for (existing_id, existing_fields) in existing_map {
                            if !kept_ids.contains(&existing_id) {
                                if rel.on_delete == OnDelete::Nilify {
                                    let mut updated = existing_fields.clone();
                                    updated.insert(child_fk.to_string(), Value::Null);
                                    if let Some(update_act) = child_update_action {
                                        Box::pin(update_dynamic(ctx, dest_def, update_act, existing_id, updated)).await?;
                                    } else {
                                        ctx.data.update(dest_def, existing_id, updated).await?;
                                    }
                                } else if let Some(destroy_act) = child_destroy_action {
                                    Box::pin(destroy_dynamic(ctx, dest_def, destroy_act, existing_id, &existing_fields)).await?;
                                } else {
                                    ctx.data.destroy(dest_def, existing_id).await?;
                                }
                            }
                        }
                    }
                }
            }
            RelKind::ManyToMany => {
                if let Some(through_fn) = rel.through {
                    let through_def = through_fn();
                    let dest_def = (rel.destination)();
                    let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                    let dest_fk = rel.destination_attribute_on_join_resource.unwrap_or("dest_id");
                    let join_pk = pk_name(through_def)?;
                    let dest_pk = pk_name(dest_def)?;

                    match managed.rel_type {
                        ManagedRelType::DirectControl => {
                            let join_filter = Filter::eq(source_fk, Value::from(parent_id));
                            let existing_joins = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(join_filter),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;

                            let mut existing_join_map = HashMap::new();
                            for j_row in existing_joins {
                                if let Some(target_val) = j_row.get(dest_fk)
                                    && let Ok(join_id) = required_uuid(&j_row, join_pk)
                                    && let Some(target_uuid) = match target_val {
                                        Value::Uuid(u) => Some(*u),
                                        Value::String(s) => Uuid::parse_str(s).ok(),
                                        _ => None,
                                    }
                                {
                                    existing_join_map.insert(target_uuid, (join_id, j_row));
                                }
                            }

                            let mut kept_targets = HashSet::new();
                            for mut child_fields in managed.inputs {
                                let target_id = if let Ok(id) = required_uuid(&child_fields, dest_pk) {
                                    id
                                } else {
                                    let create_act = dest_def
                                        .actions
                                        .iter()
                                        .find(|a| a.kind == ActionKind::Create && a.primary)
                                        .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                                    if let Some(create_act) = create_act {
                                        let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                        required_uuid(&stored, dest_pk)?
                                    } else {
                                        generate_pk(dest_def, &mut child_fields);
                                        let child_id = required_uuid(&child_fields, dest_pk)?;
                                        ctx.data.create(dest_def, child_id, child_fields).await?;
                                        child_id
                                    }
                                };

                                kept_targets.insert(target_id);
                                if !existing_join_map.contains_key(&target_id) {
                                    let mut join_fields = FieldMap::new();
                                    join_fields.insert(source_fk.to_string(), Value::from(parent_id));
                                    join_fields.insert(dest_fk.to_string(), Value::from(target_id));
                                    generate_pk(through_def, &mut join_fields);
                                    let join_id = required_uuid(&join_fields, join_pk)?;
                                    ctx.data.create(through_def, join_id, join_fields).await?;
                                }
                            }

                            // Destroy join rows for omitted targets
                            let join_destroy_action = through_def
                                .actions
                                .iter()
                                .find(|a| a.kind == ActionKind::Destroy && a.primary)
                                .or_else(|| through_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                            for (target_id, (join_id, join_row)) in existing_join_map {
                                if !kept_targets.contains(&target_id) {
                                    if let Some(destroy_act) = join_destroy_action {
                                        Box::pin(destroy_dynamic(ctx, through_def, destroy_act, join_id, &join_row)).await?;
                                    } else {
                                        ctx.data.destroy(through_def, join_id).await?;
                                    }
                                }
                            }
                        }
                        ManagedRelType::Create | ManagedRelType::Append => {
                            for mut child_fields in managed.inputs {
                                let target_id = if let Ok(id) = required_uuid(&child_fields, dest_pk) {
                                    id
                                } else {
                                    let create_act = dest_def
                                        .actions
                                        .iter()
                                        .find(|a| a.kind == ActionKind::Create && a.primary)
                                        .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                                    if let Some(create_act) = create_act {
                                        let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                        required_uuid(&stored, dest_pk)?
                                    } else {
                                        generate_pk(dest_def, &mut child_fields);
                                        let child_id = required_uuid(&child_fields, dest_pk)?;
                                        ctx.data.create(dest_def, child_id, child_fields).await?;
                                        child_id
                                    }
                                };

                                let mut join_fields = FieldMap::new();
                                join_fields.insert(source_fk.to_string(), Value::from(parent_id));
                                join_fields.insert(dest_fk.to_string(), Value::from(target_id));
                                generate_pk(through_def, &mut join_fields);
                                let join_id = required_uuid(&join_fields, join_pk)?;
                                ctx.data.create(through_def, join_id, join_fields).await?;
                            }
                        }
                    }
                }
            }
            RelKind::BelongsTo => {}
        }
    }
    Ok(())
}

/// Generic action: policies run, then `f`. No persist.
pub async fn run<R, D, T, F, Fut>(ctx: &Context<D>, action: &str, f: F) -> Result<T>
where
    R: Resource,
    D: DataLayer,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let action = action_named(&R::DEF, action)?;
    expect_kind(action, ActionKind::Generic)?;
    authorize_write(&R::DEF, action, ctx.actor.as_ref(), None)?;
    f().await
}

/// Create that runs the changeset pipeline, then a persist callback instead of the data layer.
pub async fn manual_create<R, D, F, Fut>(
    ctx: &Context<D>,
    action: &str,
    input: FieldMap,
    persist: F,
) -> Result<R>
where
    R: Resource,
    D: DataLayer,
    F: FnOnce(&Context<D>, R) -> Fut,
    Fut: Future<Output = Result<R>>,
{
    let changeset = Changeset::<R>::for_create(ctx, action, input)?;
    expect_persist(changeset.action(), PersistKind::Manual)?;
    changeset::authorize(&changeset, ctx)?;
    let record = changeset.into_record()?;
    persist(ctx, record).await
}

/// Data-layer insert with no action pipeline. Manual persist uses this to also store locally.
pub async fn insert<R: Resource, D: DataLayer>(ctx: &Context<D>, record: &R) -> Result<R> {
    let fields = record.to_fields();
    let id = required_uuid(&fields, pk_name(&R::DEF)?)?;
    let stored = ctx.data.create(&R::DEF, id, fields).await?;
    R::from_fields(&stored)
}
