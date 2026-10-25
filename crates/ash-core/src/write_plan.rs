//! Core-owned authority for an upsert's insert and conflict branches.

use crate::pipeline::{apply_tenant_scope, apply_tenant_to_fields};
use crate::{ActionDef, Context, Error, FieldMap, Filter, IdentityDef, ResourceDef, Result, Value};

/// An executable upsert, built by core after action preparation. Backends may inspect
/// the plan but cannot replace its resource, tenant, conflict guard or changed columns.
/// A conflict that fails `conflict_filter` must return [`Error::NotFound`] without a row.
///
/// ```compile_fail,E0624
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef,
///          action: &ash_core::ActionDef, identity: &ash_core::IdentityDef) {
///     let _ = ash_core::UpsertPlan::build(ctx, resource, action, identity,
///         ash_core::FieldMap::new(), &[]);
/// }
/// ```
pub struct UpsertPlan<'a> {
    scope: crate::PlanScope,
    resource: &'a ResourceDef,
    identity: &'a IdentityDef,
    tenant: Option<String>,
    id: Value,
    fields: FieldMap,
    updates: FieldMap,
    conflict_filter: Filter,
}

impl<'a> UpsertPlan<'a> {
    pub(crate) fn build<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        action: &ActionDef,
        identity: &'a IdentityDef,
        fields: FieldMap,
        update_fields: &[String],
    ) -> Result<Self> {
        let policy = crate::policy::write_filter(resource, action, ctx.authorization_actor())?;
        let lock = crate::engine::lock_of(action, &fields);
        let guard = match &lock {
            Some(lock) => Filter::and([policy, lock.guard.clone()]),
            None => policy,
        };
        let mut plan = Self::scoped(
            ctx,
            resource,
            ctx.tenant(),
            fields,
            identity,
            update_fields,
            guard,
        )?;
        // Empty update lists change no columns, including lock versions.
        if !update_fields.is_empty()
            && let Some(lock) = lock
        {
            plan.updates.extend(lock.next);
        }
        Ok(plan)
    }

    fn scoped<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        tenant: Option<&str>,
        mut fields: FieldMap,
        identity: &'a IdentityDef,
        update_fields: &[String],
        guard: Filter,
    ) -> Result<Self> {
        apply_tenant_to_fields(resource, &mut fields, tenant, true)?;
        let (filter, tenant) =
            apply_tenant_scope(resource, Some(guard), tenant.map(str::to_owned))?;
        let mut updates = FieldMap::new();
        for name in update_fields {
            let attr = resource.attribute(name).ok_or_else(|| {
                Error::Invalid(format!(
                    "unknown upsert update field `{name}` on {}",
                    resource.name
                ))
            })?;
            if attr.primary_key
                || matches!(resource.multitenancy.map(|mt| mt.strategy),
                    Some(crate::MultitenancyStrategy::Attribute(tenant)) if tenant == name)
            {
                return Err(Error::Invalid(format!(
                    "upsert cannot update identity column `{name}`"
                )));
            }
            let value = fields.get(name).ok_or_else(|| {
                Error::Invalid(format!("missing upsert update value for `{name}`"))
            })?;
            updates.insert(name.clone(), value.clone());
        }
        if let Some(rule) = crate::CredentialVersionRule::for_resource(resource) {
            updates.remove(rule.version_field);
        }
        crate::CredentialVersionRule::initialize(resource, &mut fields);
        let id = crate::pipeline::new_pk(resource, &fields)?;
        Ok(Self {
            scope: crate::PlanScope::new(ctx),
            resource,
            identity,
            tenant,
            id,
            fields,
            updates,
            conflict_filter: filter.unwrap_or(Filter::True),
        })
    }

    pub fn scope(&self) -> &crate::PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn identity(&self) -> &IdentityDef {
        self.identity
    }
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
    pub fn id(&self) -> &Value {
        &self.id
    }
    pub fn fields(&self) -> &FieldMap {
        &self.fields
    }
    pub fn updates(&self) -> &FieldMap {
        &self.updates
    }
    pub fn conflict_filter(&self) -> &Filter {
        &self.conflict_filter
    }
}
