//! Writes an action's optimistic lock guards, as Ash's data layers apply a changeset's
//! filter to its update or destroy: the write lands only if the record still holds the
//! lock versions it was read at. A record that has moved on since is stale; one that's
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

/// What `action`'s optimistic locks put on writing a record read as `existing`.
#[derive(Clone, Debug)]
pub(crate) struct Lock {
    /// Each lock attribute still holding the value read, as Ash's `optimistic_lock`
    /// filters the write.
    pub guard: Filter,
    /// Each lock attribute's next value, one more than the value read. An update writes
    /// them last, over whatever the action's changes set, as Ash's increment is set in a
    /// before-action hook, after every change.
    pub next: FieldMap,
}

/// The lock `action`'s optimistic locks put on writing `existing`, if it has any.
pub(crate) fn lock_of(action: &ActionDef, existing: &FieldMap) -> Option<Lock> {
    let mut guards = Vec::new();
    let mut next = FieldMap::new();
    for field in action.optimistic_locks() {
        guards.push(match existing.get(field) {
            None | Some(Value::Null) => Filter::is_nil(field),
            Some(value) => Filter::eq(field, value.clone()),
        });
        next.insert(field.to_string(), next_version(existing.get(field)));
    }
    match guards.len() {
        0 => None,
        1 => Some(Lock { guard: guards.pop().expect("one guard"), next }),
        _ => Some(Lock { guard: Filter::and(guards), next }),
    }
}

/// The version after `current`: one more, or 1 from none (Ash's lock needs one to add
/// to; a null version is taken as 0).
pub(crate) fn next_version(current: Option<&Value>) -> Value {
    match current {
        Some(Value::Int(n)) => Value::Int(n + 1),
        _ => Value::Int(1),
    }
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

/// Checks record `id` still holds the versions `lock` read, before anything that can't
/// be taken back runs on its behalf (a cascade to its related records), as Ash's filter
/// leaves a stale record out before its action runs.
pub(crate) async fn check_lock<D: DataLayer>(ctx: &Context<D>, resource: &'static ResourceDef, id: Value, lock: &Lock) -> Result<()> {
    let query = guarded_query(ctx.tenant.clone(), resource, &id, lock.guard.clone())?;
    if ctx.data.run_query(resource, &query).await?.is_empty() {
        return Err(stale_or_missing(ctx, resource, id).await);
    }
    Ok(())
}

/// Writes `fields` to record `id`, under `lock` when the action locks: its next versions
/// set last, and the write filtered to the versions read, as one statement where the
/// data layer updates atomically, else reading the record to check it first.
pub(crate) async fn update_guarded<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    id: Value,
    mut fields: FieldMap,
    lock: Option<Lock>,
) -> Result<FieldMap> {
    let Some(lock) = lock else {
        return ctx.data.update(resource, ctx.tenant.as_deref(), id, fields).await;
    };
    fields.extend(lock.next);
    let query = guarded_query(ctx.tenant.clone(), resource, &id, lock.guard)?;
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

/// Deletes record `id`, under `lock` when the action locks, as [`update_guarded`]
/// writes.
pub(crate) async fn destroy_guarded<D: DataLayer>(ctx: &Context<D>, resource: &'static ResourceDef, id: Value, lock: Option<&Lock>) -> Result<()> {
    let Some(lock) = lock else {
        return ctx.data.destroy(resource, ctx.tenant.as_deref(), id).await;
    };
    let query = guarded_query(ctx.tenant.clone(), resource, &id, lock.guard.clone())?;
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
