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
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

/// Server configuration allowing credential reads of one resource without its read
/// policies or redaction. Authentication must inspect hashes before an actor exists.
/// This permit never bypasses tenancy. Do not construct it from request input.
#[derive(Clone)]
pub struct CredentialReadPermit {
    resource: &'static ResourceDef,
}

impl CredentialReadPermit {
    /// Grant trusted server code credential access to this resource. Grant issuance
    /// is public until server-owned scope binding is introduced in migration step 2.
    pub fn new(resource: &'static ResourceDef) -> Self {
        Self { resource }
    }
}

/// Borrowing the context holds its actor and tenant fixed for this execution.
/// Returned maps are engine data. Callers must apply field policies before exposing
/// them. The existing public `Context::data` interface can still bypass this handle.
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
    /// Read through action preparations, row policies and tenant scoping. Field
    /// presentation remains the caller's responsibility until the view migration.
    pub async fn read(
        &self,
        resource: &ResourceDef,
        action: &str,
        arguments: &FieldMap,
        request: ReadRequest,
    ) -> Result<Vec<FieldMap>> {
        let action = crate::pipeline::read_action(resource, Some(action))?;
        let query = crate::scope_read(
            resource,
            action,
            self.ctx.actor.as_ref(),
            arguments,
            CompiledQuery {
                filter: request.filter,
                sort: request.sort,
                limit: request.limit,
                offset: request.offset,
                tenant: self.ctx.tenant.clone(),
                ..Default::default()
            },
        )?;
        let mut rows = self.ctx.data.run_query(resource, &query).await?;
        crate::after_read(action, arguments, &mut rows)?;
        Ok(rows)
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
            self.ctx.tenant.clone(),
        )?;
        self.ctx
            .data
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
