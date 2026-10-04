//! Writes an action's optimistic lock guards, as Ash's data layers apply a changeset's
//! filter to its update or destroy: the write lands only if the record still holds the
//! lock version it was read at. A record that has moved on since is stale; one that's
//! gone isn't found.

use crate::action::ActionDef;
use crate::atomic::{AtomicExpr, AtomicUpdate};
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{apply_tenant_scope, pk_name};
use crate::resource::ResourceDef;
use crate::value::{FieldMap, Value};

/// The guard `action`'s optimistic lock puts on writing `existing`: its lock attribute
/// still holding the value read, as Ash's `optimistic_lock` filters the write.
pub(crate) fn lock_guard(action: &ActionDef, existing: &FieldMap) -> Option<Filter> {
    let field = action.optimistic_lock()?;
    Some(match existing.get(field) {
        None | Some(Value::Null) => Filter::is_nil(field),
        Some(value) => Filter::eq(field, value.clone()),
    })
}

/// Record `id` within the context's tenant and `guard`.
fn guarded_query(ctx_tenant: Option<String>, resource: &ResourceDef, id: &Value, guard: Filter) -> Result<CompiledQuery> {
    let by_id = Filter::and([Filter::eq(pk_name(resource)?, id.clone()), guard]);
    let (filter, tenant) = apply_tenant_scope(resource, Some(by_id), ctx_tenant)?;
    Ok(CompiledQuery { filter, tenant, limit: Some(1), ..CompiledQuery::default() })
}

/// Why a guarded write of record `id` found nothing to write: it's still there, so it
/// changed since it was read ([`Error::StaleRecord`]), or it isn't ([`Error::NotFound`]).
async fn stale_or_missing<D: DataLayer>(ctx: &Context<D>, resource: &ResourceDef, id: Value) -> Error {
    let found = match pk_name(resource).and_then(|pk| apply_tenant_scope(resource, Some(Filter::eq(pk, id.clone())), ctx.tenant.clone())) {
        Ok((filter, tenant)) => ctx.data.run_query(resource, &CompiledQuery { filter, tenant, limit: Some(1), ..CompiledQuery::default() }).await,
        Err(err) => return err,
    };
    match found {
        Ok(rows) if !rows.is_empty() => Error::StaleRecord { resource: resource.name, id },
        Ok(_) => Error::NotFound,
        Err(err) => err,
    }
}

/// Writes `fields` to record `id`, within `guard` when the action locks: as one statement
/// where the data layer updates atomically, else reading the record to check it first.
pub(crate) async fn update_guarded<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    id: Value,
    fields: FieldMap,
    guard: Option<Filter>,
) -> Result<FieldMap> {
    let Some(guard) = guard else {
        return ctx.data.update(resource, ctx.tenant.as_deref(), id, fields).await;
    };
    let query = guarded_query(ctx.tenant.clone(), resource, &id, guard)?;
    if ctx.data.can_update_atomically(resource) {
        let mut update = AtomicUpdate::default();
        for (name, value) in fields {
            update.set(name, AtomicExpr::Value(value));
        }
        return match ctx.data.update_atomic(resource, &query, &update).await?.pop() {
            Some(stored) => Ok(stored),
            None => Err(stale_or_missing(ctx, resource, id).await),
        };
    }
    if ctx.data.run_query(resource, &query).await?.is_empty() {
        return Err(stale_or_missing(ctx, resource, id).await);
    }
    ctx.data.update(resource, ctx.tenant.as_deref(), id, fields).await
}

/// Deletes record `id`, within `guard` when the action locks, as [`update_guarded`]
/// writes.
pub(crate) async fn destroy_guarded<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    id: Value,
    guard: Option<Filter>,
) -> Result<()> {
    let Some(guard) = guard else {
        return ctx.data.destroy(resource, ctx.tenant.as_deref(), id).await;
    };
    let query = guarded_query(ctx.tenant.clone(), resource, &id, guard)?;
    if ctx.data.can_destroy_atomically(resource) {
        return match ctx.data.destroy_atomic(resource, &query, &[]).await?.is_empty() {
            false => Ok(()),
            true => Err(stale_or_missing(ctx, resource, id).await),
        };
    }
    if ctx.data.run_query(resource, &query).await?.is_empty() {
        return Err(stale_or_missing(ctx, resource, id).await);
    }
    ctx.data.destroy(resource, ctx.tenant.as_deref(), id).await
}
