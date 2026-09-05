use std::future::Future;
use std::marker::PhantomData;

use uuid::Uuid;

use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::keys::{AggregateName, CalcName, FieldName, RelName};
use crate::pipeline::{pk_name, read_action};
use crate::policy::compile_read_filter;
use crate::resource::Resource;
use crate::value::{FieldMap, Value};

use super::lifecycle::get;
use super::pagination::{build_keyset_filter, cursor_for_record, KeysetCursor, Page};
use super::relations::attach_relationships;

pub struct Query<'a, R, D> {
    ctx: &'a Context<D>,
    action: Option<&'static str>,
    pub arguments: FieldMap,
    filter: Option<Filter>,
    sort: Vec<Sort>,
    calculations: Vec<String>,
    calculation_args: std::collections::HashMap<String, FieldMap>,
    aggregates: Vec<String>,
    loads: Vec<String>,
    limit: Option<usize>,
    offset: Option<usize>,
    tenant: Option<String>,
    _resource: PhantomData<R>,
}

impl<'a, R, D> Clone for Query<'a, R, D> {
    fn clone(&self) -> Self {
        Self {
            ctx: self.ctx,
            action: self.action,
            arguments: self.arguments.clone(),
            filter: self.filter.clone(),
            sort: self.sort.clone(),
            calculations: self.calculations.clone(),
            calculation_args: self.calculation_args.clone(),
            aggregates: self.aggregates.clone(),
            loads: self.loads.clone(),
            limit: self.limit,
            offset: self.offset,
            tenant: self.tenant.clone(),
            _resource: PhantomData,
        }
    }
}

impl<'a, R: Resource, D: DataLayer> Query<'a, R, D> {
    pub fn action(mut self, name: &'static str) -> Self {
        self.action = Some(name);
        self
    }

    /// Set an action argument on this query.
    pub fn argument(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.arguments.insert(name.into(), value.into());
        self
    }

    /// Set multiple action arguments on this query.
    pub fn arguments(mut self, args: FieldMap) -> Self {
        self.arguments.extend(args);
        self
    }

    /// Set or override the tenant on this query.
    pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Clear the tenant on this query.
    pub fn without_tenant(mut self) -> Self {
        self.tenant = None;
        self
    }

    /// Retrieve the current tenant on this query, if set.
    pub fn get_tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
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

    pub fn calc_with_args(mut self, name: impl CalcName<R>, args: FieldMap) -> Self {
        let calc_name = name.as_calc().to_string();
        self.calculation_args.insert(calc_name.clone(), args);
        self.calculations.push(calc_name);
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
                crate::action::PreparationDef::FilterWithArgs(filter_fn) => {
                    let prep_filter = filter_fn(&self.arguments);
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
        let mut this = self.apply_preparations(action);

        for (arg_name, arg_val) in &this.arguments {
            if R::DEF.attributes.iter().any(|a| a.name == arg_name.as_str()) {
                let attr_filter = Filter::eq(arg_name.as_str(), arg_val.clone());
                this.filter = match this.filter {
                    Some(f) => Some(Filter::and([f, attr_filter])),
                    None => Some(attr_filter),
                };
            }
        }

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

        let tenant = this.tenant.clone().or_else(|| this.ctx.tenant().map(|s| s.to_string()));
        let mut filter = filter;

        if let Some(mt) = R::DEF.multitenancy {
            match mt.strategy {
                crate::resource::MultitenancyStrategy::Attribute(attr_name) => {
                    if let Some(ref t) = tenant {
                        let tenant_filter = Filter::eq(attr_name, Value::String(t.clone()));
                        filter = match filter {
                            Some(existing) => Some(Filter::and([existing, tenant_filter])),
                            None => Some(tenant_filter),
                        };
                    } else if !mt.global {
                        return Err(Error::TenantRequired { resource: R::DEF.name });
                    }
                }
                crate::resource::MultitenancyStrategy::Context => {
                    if tenant.is_none() && !mt.global {
                        return Err(Error::TenantRequired { resource: R::DEF.name });
                    }
                }
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
                    calculation_args: this.calculation_args,
                    aggregates: this.aggregates,
                    limit: this.limit,
                    offset: this.offset,
                    tenant,
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
                    calculation_args: std::collections::HashMap::new(),
                    aggregates: Vec::new(),
                    limit: None,
                    offset: None,
                    tenant: this.tenant,
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
                arguments: self.arguments.clone(),
                filter: self.filter.clone(),
                sort: Vec::new(),
                calculations: Vec::new(),
                calculation_args: std::collections::HashMap::new(),
                aggregates: Vec::new(),
                loads: Vec::new(),
                limit: None,
                offset: None,
                tenant: self.tenant.clone(),
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
                let mut val = cursor
                    .values
                    .iter()
                    .find(|(k, _)| k == &s.field)
                    .map(|(_, v)| v.clone())
                    .unwrap_or(Value::Null);
                if val.is_null() && s.field == pk {
                    val = Value::Uuid(cursor.id);
                }
                sort_tuples.push((s.field.clone(), val, s.descending));
            }

            if let Some(keyset_filter) = build_keyset_filter(&sort_tuples, !is_before) {
                self = self.filter(keyset_filter);
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

    /// Batch destroys all records matching this query.
    pub async fn bulk_destroy(
        self,
        action: &'static str,
        opts: crate::bulk::BulkDestroyOptions,
    ) -> Result<crate::bulk::BulkResult<R>> {
        let records = self.clone().load().await?;
        let ids: Vec<Uuid> = records.iter().map(Resource::id).collect();
        crate::bulk::bulk_destroy::<R, D>(self.ctx, action, &ids, opts).await
    }

    /// Chunked streaming over offset-based pagination for large datasets.
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
        let mut offset = 0;
        let mut total = 0;

        loop {
            let page = self
                .clone()
                .page_offset(chunk_size, offset, false)
                .await?;

            let count = page.results.len();
            if count == 0 {
                break;
            }

            total += count;
            offset += count;
            let has_more = page.has_more;

            handler(page.results).await?;

            if !has_more {
                break;
            }
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

pub fn query<R: Resource, D: DataLayer>(ctx: &Context<D>) -> Query<'_, R, D> {
    Query {
        ctx,
        action: None,
        arguments: FieldMap::new(),
        filter: None,
        sort: Vec::new(),
        calculations: Vec::new(),
        calculation_args: std::collections::HashMap::new(),
        aggregates: Vec::new(),
        loads: Vec::new(),
        limit: None,
        offset: None,
        tenant: ctx.tenant.clone(),
        _resource: PhantomData,
    }
}
