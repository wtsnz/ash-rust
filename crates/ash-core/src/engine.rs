use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;

use uuid::Uuid;

use crate::action::{ActionKind, PersistKind};
use crate::changeset::{self, Changeset};
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::keys::{AggregateName, CalcName, FieldName, RelName};
use crate::pipeline::{action_named, expect_kind, expect_persist, pk_name, read_action};
use crate::policy::{authorize_write, compile_read_filter};
use crate::resource::{RelKind, Resource, ResourceDef};
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
    authorize_write(
        &R::DEF,
        action_def,
        ctx.actor.as_ref(),
        Some(&existing_fields),
    )?;
    ctx.data.destroy(&R::DEF, id).await?;

    let notification = crate::notifier::Notification::new(
        R::DEF.name,
        action_def.name,
        ActionKind::Destroy,
        id,
        existing_fields.clone(),
        Some(existing_fields),
        ctx.actor.clone(),
        crate::value::FieldMap::new(),
    );
    crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;

    Ok(())
}

pub async fn destroy_existing<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    existing: R,
) -> Result<()> {
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Destroy)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    let id = existing.id();
    let existing_fields = existing.to_fields();
    authorize_write(
        &R::DEF,
        action_def,
        ctx.actor.as_ref(),
        Some(&existing_fields),
    )?;
    ctx.data.destroy(&R::DEF, id).await?;

    let notification = crate::notifier::Notification::new(
        R::DEF.name,
        action_def.name,
        ActionKind::Destroy,
        id,
        existing_fields.clone(),
        Some(existing_fields),
        ctx.actor.clone(),
        crate::value::FieldMap::new(),
    );
    crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;

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
