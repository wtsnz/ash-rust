//! Scope-bound reads. Callers supply query shape, never a second actor or tenant.

use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, Sort};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::ResourceDef;
use crate::value::FieldMap;

/// The caller-controlled shape of a read. Execution identity belongs to the handle.
///
/// ```compile_fail
/// let request = ash_core::ReadRequest {
///     tenant: Some("another-tenant".into()),
///     ..Default::default()
/// };
/// ```
#[derive(Clone, Debug, Default)]
pub struct ReadRequest {
    pub lock: Option<crate::ReadLock>,
    pub distinct_on: Option<crate::DistinctOn>,
    pub select: Option<Vec<String>>,
    pub calculations: Vec<String>,
    pub calculation_args: std::collections::HashMap<String, FieldMap>,
    pub aggregates: Vec<String>,
    /// Seal cursor values before read hooks and projection, in this effective ordering.
    pub keyset: Option<Vec<Sort>>,
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// A named read prepared by core for one exact request scope. Extensions may use
/// its query shape to paginate, but cannot replace its candidate predicate or identity.
///
/// ```compile_fail,E0616
/// fn forge(read: &mut ash_core::PreparedRead<'_>) {
///     read.query = ash_core::CompiledQuery::default();
/// }
/// ```
#[derive(Clone)]
pub struct PreparedRead<'a> {
    scope: std::sync::Arc<crate::context::Scope>,
    resource: &'a ResourceDef,
    action: &'a crate::ActionDef,
    arguments: FieldMap,
    query: CompiledQuery,
}
impl PreparedRead<'_> {
    pub fn query(&self) -> &CompiledQuery {
        &self.query
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
}

/// Server configuration allowing credential reads of one resource without its read
/// policies or redaction. Authentication must inspect hashes before an actor exists.
/// Runtime issues this permit at server bootstrap for its own contexts. It never
/// bypasses tenancy, and an unrelated Runtime cannot grant access to this context.
///
/// ```compile_fail
/// fn forge(resource: &'static ash_core::ResourceDef) {
///     let _ = ash_core::CredentialReadPermit { resource };
/// }
/// ```
#[derive(Clone)]
pub struct CredentialReadPermit {
    pub(crate) resource: &'static ResourceDef,
    pub(crate) authority: std::sync::Arc<crate::context::RuntimeAuthority>,
}

/// Bootstrap grant to mutate named credential attributes on one resource.
/// It applies only to the issuing Runtime's contexts, cannot be issued from a request
/// Context and never bypasses tenancy.
///
/// ```compile_fail
/// fn forge(resource: &'static ash_core::ResourceDef) {
///     let _ = ash_core::CredentialWritePermit { resource, fields: Vec::new() };
/// }
/// ```
#[derive(Clone)]
pub struct CredentialWritePermit {
    pub(crate) resource: &'static ResourceDef,
    pub(crate) fields: Vec<&'static str>,
    pub(crate) authority: std::sync::Arc<crate::context::RuntimeAuthority>,
}

/// Borrowing the context holds its actor and tenant fixed for this execution.
/// Ordinary reads return reader-specific Views. Credential reads remain privileged
/// engine data available only through an explicit credential grant.
pub struct ScopedAccess<'a, D> {
    ctx: &'a Context<D>,
}

impl<D> Context<D> {
    pub fn access(&self) -> ScopedAccess<'_, D> {
        ScopedAccess { ctx: self }
    }
}

impl<D> ScopedAccess<'_, D> {
    /// Check a signed credential's tenant, including the absence of a tenant.
    /// Call before consuming a token or changing a password.
    pub fn require_tenant_match(&self, tenant: Option<&str>) -> Result<()> {
        if tenant == self.ctx.tenant() {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}

impl<D: DataLayer> ScopedAccess<'_, D> {
    /// Resolve preparations, policy predicates and tenancy once before pagination.
    pub fn prepare_read<'a>(
        &self,
        resource: &'a ResourceDef,
        action: &str,
        arguments: &FieldMap,
        request: ReadRequest,
    ) -> Result<PreparedRead<'a>> {
        let action = crate::pipeline::read_action(resource, Some(action))?;
        let query = self.plan_read(resource, action, arguments, &request)?;
        Ok(PreparedRead {
            scope: std::sync::Arc::clone(&self.ctx.scope),
            resource,
            action,
            arguments: arguments.clone(),
            query,
        })
    }

    fn prepared_query(
        &self,
        read: &PreparedRead<'_>,
        request: &ReadRequest,
    ) -> Result<CompiledQuery> {
        if !std::sync::Arc::ptr_eq(&read.scope, &self.ctx.scope) {
            return Err(Error::Invalid(
                "read belongs to a different request scope".into(),
            ));
        }
        let filter = crate::plan_filter(
            read.resource,
            self.ctx.authorization_actor(),
            self.ctx.tenant(),
            crate::TrustedExpr::Filter(request.filter.clone().unwrap_or(Filter::True)),
        )?;
        Ok(CompiledQuery {
            filter: crate::pipeline::and_filters(read.query.filter.clone(), Some(filter)),
            lock: request.lock,
            distinct_on: request.distinct_on.clone(),
            sort: request.sort.clone(),
            select: request.select.clone(),
            calculations: request.calculations.clone(),
            calculation_args: request.calculation_args.clone(),
            aggregates: request.aggregates.clone(),
            limit: request.limit,
            offset: request.offset,
            actor: read.query.actor.clone(),
            tenant: read.query.tenant.clone(),
        })
    }

    /// Apply page shape without dropping the prepared read's mandatory predicates.
    pub async fn read_prepared(
        &self,
        read: &PreparedRead<'_>,
        request: ReadRequest,
    ) -> Result<Vec<crate::View>> {
        let query = self.prepared_query(read, &request)?;
        let rows = self
            .ctx
            .raw_data()
            .run_query(&crate::QueryPlan::new(self.ctx, read.resource, &query))
            .await?;
        self.finish_read(read.resource, read.action, &read.arguments, &request, rows)
    }

    pub async fn count_prepared(
        &self,
        read: &PreparedRead<'_>,
        request: ReadRequest,
    ) -> Result<usize> {
        let query = self.prepared_query(read, &request)?;
        self.ctx
            .raw_data()
            .count(&crate::CountPlan::new(self.ctx, read.resource, &query))
            .await
    }

    pub async fn read_prepared_with_count(
        &self,
        read: &PreparedRead<'_>,
        page: ReadRequest,
        count: ReadRequest,
    ) -> Result<(Vec<crate::View>, usize)> {
        let query = self.prepared_query(read, &page)?;
        let count_query = self.prepared_query(read, &count)?;
        let (rows, count) = self
            .ctx
            .raw_data()
            .run_query_with_count(&crate::QueryWithCountPlan::new(
                self.ctx,
                read.resource,
                &query,
                &count_query,
            ))
            .await?;
        Ok((
            self.finish_read(read.resource, read.action, &read.arguments, &page, rows)?,
            count,
        ))
    }

    /// Read through action preparations, row policies and tenant scoping, then
    /// present every record for this context's reader.
    pub async fn read(
        &self,
        resource: &ResourceDef,
        action: &str,
        arguments: &FieldMap,
        request: ReadRequest,
    ) -> Result<Vec<crate::View>> {
        let action = crate::pipeline::read_action(resource, Some(action))?;
        let query = self.plan_read(resource, action, arguments, &request)?;
        let rows = self
            .ctx
            .raw_data()
            .run_query(&crate::QueryPlan::new(self.ctx, resource, &query))
            .await?;
        self.finish_read(resource, action, arguments, &request, rows)
    }

    /// Count through the same read planner as ordinary reads.
    pub async fn count(
        &self,
        resource: &ResourceDef,
        action: &str,
        arguments: &FieldMap,
        request: ReadRequest,
    ) -> Result<usize> {
        let action = crate::pipeline::read_action(resource, Some(action))?;
        let query = self.plan_read(resource, action, arguments, &request)?;
        self.ctx
            .raw_data()
            .count(&crate::CountPlan::new(self.ctx, resource, &query))
            .await
    }

    /// Read a page and its count together, then present the page inside core.
    pub async fn read_with_count(
        &self,
        resource: &ResourceDef,
        action: &str,
        arguments: &FieldMap,
        page: ReadRequest,
        count: ReadRequest,
    ) -> Result<(Vec<crate::View>, usize)> {
        let action = crate::pipeline::read_action(resource, Some(action))?;
        let page_query = self.plan_read(resource, action, arguments, &page)?;
        let count_query = self.plan_read(resource, action, arguments, &count)?;
        let (rows, count) = self
            .ctx
            .raw_data()
            .run_query_with_count(&crate::QueryWithCountPlan::new(
                self.ctx,
                resource,
                &page_query,
                &count_query,
            ))
            .await?;
        Ok((
            self.finish_read(resource, action, arguments, &page, rows)?,
            count,
        ))
    }

    fn plan_read(
        &self,
        resource: &ResourceDef,
        action: &crate::ActionDef,
        arguments: &FieldMap,
        request: &ReadRequest,
    ) -> Result<CompiledQuery> {
        crate::scope_read(
            resource,
            action,
            self.ctx.authorization_actor(),
            arguments,
            CompiledQuery {
                lock: request.lock,
                distinct_on: request.distinct_on.clone(),
                select: request.select.clone(),
                calculations: request.calculations.clone(),
                calculation_args: request.calculation_args.clone(),
                aggregates: request.aggregates.clone(),
                filter: request.filter.clone(),
                sort: request.sort.clone(),
                limit: request.limit,
                offset: request.offset,
                tenant: self.ctx.tenant().map(str::to_owned),
                ..Default::default()
            },
        )
    }

    fn finish_read(
        &self,
        resource: &ResourceDef,
        action: &crate::ActionDef,
        arguments: &FieldMap,
        request: &ReadRequest,
        mut rows: Vec<FieldMap>,
    ) -> Result<Vec<crate::View>> {
        let cursors = rows
            .iter()
            .map(|row| {
                request
                    .keyset
                    .as_ref()
                    .map(|sort| {
                        self.ctx.seal_keyset_with_distinct(
                            resource,
                            action.name,
                            sort,
                            request.distinct_on.as_ref(),
                            row,
                        )
                    })
                    .transpose()
            })
            .collect::<Result<Vec<_>>>()?;
        crate::after_read(action, arguments, &mut rows)?;
        rows.into_iter()
            .zip(cursors)
            .map(|(row, cursor)| {
                let mut view = self.ctx.present(resource, row.into(), None)?;
                view.set_keyset(cursor);
                Ok(view)
            })
            .collect()
    }

    /// Engine-only discovery retains stored keys and policy operands until the write.
    /// Hard cascades include archived rows, retaining other preparations and policies.
    pub(crate) async fn related_rows(
        &self,
        resource: &ResourceDef,
        filter: Filter,
        hard: bool,
    ) -> Result<Vec<FieldMap>> {
        let action = crate::pipeline::read_action(resource, None)?;
        let query = crate::engine::scope_discovery(
            resource,
            action,
            self.ctx.authorization_actor(),
            &FieldMap::new(),
            CompiledQuery {
                filter: Some(filter),
                tenant: self.ctx.tenant().map(str::to_owned),
                ..Default::default()
            },
            hard,
        )?;
        let mut rows = self
            .ctx
            .raw_data()
            .run_query(&crate::QueryPlan::new(self.ctx, resource, &query))
            .await?;
        crate::after_read(action, &FieldMap::new(), &mut rows)?;
        Ok(rows)
    }

    pub(crate) async fn resolve_related(
        &self,
        resource: &ResourceDef,
        id: crate::Value,
    ) -> Result<FieldMap> {
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let id = crate::value::pk_cast(pk.ty, &id).ok_or(Error::NotFound)?;
        crate::pipeline::check_type(pk, &id).map_err(|_| Error::NotFound)?;
        self.related_rows(
            resource,
            Filter::eq(crate::pipeline::pk_name(resource)?, id),
            false,
        )
        .await
        .map_err(|error| match error {
            Error::Forbidden => Error::NotFound,
            other => other,
        })?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)
    }

    /// Update only the credential fields named by a bootstrap-issued grant.
    /// Authentication validates the credential before calling this operation.
    pub async fn update_credentials(
        &self,
        permit: &CredentialWritePermit,
        id: crate::Value,
        mut fields: FieldMap,
    ) -> Result<crate::RawRecord> {
        if !std::sync::Arc::ptr_eq(&permit.authority, &self.ctx.scope.authority) {
            return Err(Error::Forbidden);
        }
        for name in fields.keys() {
            let attr = permit
                .resource
                .attribute(name)
                .ok_or_else(|| Error::Invalid(format!("unknown credential field `{name}`")))?;
            if !permit.fields.contains(&name.as_str())
                || attr.primary_key
                || matches!(permit.resource.multitenancy.map(|mt| mt.strategy), Some(crate::MultitenancyStrategy::Attribute(tenant)) if tenant == name)
            {
                return Err(Error::Forbidden);
            }
            crate::pipeline::check_type(attr, &fields[name])?;
        }
        crate::pipeline::apply_tenant_to_fields(
            permit.resource,
            &mut fields,
            self.ctx.tenant(),
            false,
        )?;
        let plan = crate::UpdatePlan::new(self.ctx, permit.resource, id, fields);
        self.ctx
            .raw_data()
            .update(&plan)
            .await
            .map(crate::RawRecord::from)
    }

    /// Conditionally mutate credential state in one statement. Zero returned rows means
    /// the condition lost a race. Unsupported backends fail before writing.
    pub async fn update_credentials_where(
        &self,
        permit: &CredentialWritePermit,
        filter: Filter,
        fields: FieldMap,
    ) -> Result<Vec<crate::RawRecord>> {
        if !std::sync::Arc::ptr_eq(&permit.authority, &self.ctx.scope.authority) {
            return Err(Error::Forbidden);
        }
        if !self.ctx.raw_data().can_update_atomically(permit.resource) {
            return Err(Error::AtomicityUnsupported);
        }
        for (name, value) in &fields {
            let attr = permit.resource.attribute(name).ok_or(Error::Forbidden)?;
            if !permit.fields.contains(&name.as_str())
                || attr.primary_key
                || matches!(permit.resource.multitenancy.map(|mt| mt.strategy), Some(crate::MultitenancyStrategy::Attribute(tenant)) if tenant == name)
            {
                return Err(Error::Forbidden);
            }
            crate::pipeline::check_type(attr, value)?;
        }
        let (filter, tenant) = crate::pipeline::apply_tenant_scope(
            permit.resource,
            Some(filter),
            self.ctx.tenant().map(str::to_owned),
        )?;
        let query = CompiledQuery {
            filter,
            tenant,
            ..Default::default()
        };
        let update = crate::CredentialVersionRule::update(permit.resource, &fields);
        self.ctx
            .raw_data()
            .update_atomic(&crate::AtomicUpdatePlan::new(
                self.ctx,
                permit.resource,
                &query,
                &update,
            ))
            .await
            .map(|rows| rows.into_iter().map(crate::RawRecord::from).collect())
    }

    /// Look up credentials using an explicit resource-bound server permit. There is
    /// no caller-supplied `CompiledQuery` that could forget the attribute tenant
    /// predicate or replace the schema tenant.
    pub async fn credentials(
        &self,
        permit: &CredentialReadPermit,
        filter: Filter,
        limit: usize,
    ) -> Result<Vec<FieldMap>> {
        if !std::sync::Arc::ptr_eq(&permit.authority, &self.ctx.scope.authority) {
            return Err(Error::Forbidden);
        }
        let (filter, tenant) = crate::pipeline::apply_tenant_scope(
            permit.resource,
            Some(filter),
            self.ctx.tenant().map(str::to_owned),
        )?;
        self.ctx
            .raw_data()
            .run_query(&crate::QueryPlan::new(
                self.ctx,
                permit.resource,
                &CompiledQuery {
                    filter,
                    tenant,
                    limit: Some(limit),
                    ..Default::default()
                },
            ))
            .await
    }
}
