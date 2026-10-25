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
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Server configuration allowing credential reads of one resource without its read
/// policies or redaction. Authentication must inspect hashes before an actor exists.
/// Runtime issues this permit at server bootstrap. It never bypasses tenancy.
///
/// ```compile_fail
/// fn forge(resource: &'static ash_core::ResourceDef) {
///     let _ = ash_core::CredentialReadPermit::new(resource);
/// }
/// ```
#[derive(Clone)]
pub struct CredentialReadPermit {
    resource: &'static ResourceDef,
}

impl CredentialReadPermit {
    pub(crate) fn new(resource: &'static ResourceDef) -> Self {
        Self { resource }
    }
}

/// Borrowing the context holds its actor and tenant fixed for this execution.
/// Ordinary reads return reader-specific Views. Credential reads remain privileged
/// engine data. The deprecated `Context::data` interface can still bypass this handle.
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
        let query = crate::scope_read(
            resource,
            action,
            self.ctx.authorization_actor(),
            arguments,
            CompiledQuery {
                lock: request.lock,
                distinct_on: request.distinct_on,
                filter: request.filter,
                sort: request.sort,
                limit: request.limit,
                offset: request.offset,
                tenant: self.ctx.tenant().map(str::to_owned),
                ..Default::default()
            },
        )?;
        let mut rows = self.ctx.raw_data().run_query(resource, &query).await?;
        crate::after_read(action, arguments, &mut rows)?;
        rows.into_iter()
            .map(|row| self.ctx.present(resource, row.into(), None))
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
        let mut rows = self.ctx.raw_data().run_query(resource, &query).await?;
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

    /// Look up credentials using an explicit resource-bound server permit. There is
    /// no caller-supplied `CompiledQuery` that could forget the attribute tenant
    /// predicate or replace the schema tenant.
    pub async fn credentials(
        &self,
        permit: &CredentialReadPermit,
        filter: Filter,
        limit: usize,
    ) -> Result<Vec<FieldMap>> {
        let (filter, tenant) = crate::pipeline::apply_tenant_scope(
            permit.resource,
            Some(filter),
            self.ctx.tenant().map(str::to_owned),
        )?;
        self.ctx
            .raw_data()
            .run_query(
                permit.resource,
                &CompiledQuery {
                    filter,
                    tenant,
                    limit: Some(limit),
                    ..Default::default()
                },
            )
            .await
    }
}
