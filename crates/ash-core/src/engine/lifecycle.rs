use std::future::Future;
use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, PersistKind};
use crate::changeset::{self, Changeset, DynamicChangeset};
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{expect_kind, expect_persist, pk_name};
use crate::resource::{Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

use super::query::query;

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
    let existing = get::<R, D>(ctx, id).await?;
    destroy_existing(ctx, action, existing).await
}

pub async fn destroy_existing<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    existing: R,
) -> Result<()> {
    destroy_existing_returning(ctx, action, existing).await?;
    Ok(())
}

/// [`destroy_existing`] returning the stored record, which a soft destroy has archived.
pub(crate) async fn destroy_existing_returning<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    existing: R,
) -> Result<R> {
    Changeset::for_destroy(ctx, action, existing)?.commit(ctx).await
}

pub async fn destroy_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    existing_fields: &FieldMap,
) -> Result<()> {
    let cascade = super::managed::Cascade::new(true);
    destroy_dynamic_with(ctx, resource, action, id, existing_fields, &cascade)
        .await
        .map(|_| ())
}

/// [`destroy_dynamic`] within `cascade`, which decides whether it notifies. Returns the
/// stored fields, which differ from `existing_fields` after a soft destroy.
pub(crate) async fn destroy_dynamic_with<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    existing_fields: &FieldMap,
    cascade: &super::managed::Cascade,
) -> Result<FieldMap> {
    destroy_dynamic_input(ctx, resource, action, id, existing_fields, FieldMap::new(), cascade).await
}

/// [`destroy_dynamic_with`] given `input`: the action's arguments, and attributes a soft
/// destroy writes.
async fn destroy_dynamic_input<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    existing_fields: &FieldMap,
    input: FieldMap,
    cascade: &super::managed::Cascade,
) -> Result<FieldMap> {
    let mut existing = existing_fields.clone();
    existing
        .entry(pk_name(resource)?.to_string())
        .or_insert(Value::Uuid(id));
    DynamicChangeset::for_destroy_with(ctx, resource, action, existing, input)?
        .commit_within(ctx, cascade)
        .await
}

/// A destroy through `action` of the record `id` as the context reads it, which must
/// still have lock version `expected_version` if given, else [`Error::StaleRecord`]. As
/// AshGraphql's destroy runs a bulk destroy over the record's query, it runs as one
/// statement when the data layer can and the action allows, with no read first: a hard
/// destroy as a delete, its validations, policies and the version checked in it; a soft
/// destroy as an update (failing when it must be atomic and can't be). Otherwise it reads
/// the record and destroys that. Either way the read action's policies decide which
/// records it finds, as [`update_dynamic_expecting`]'s do. Returns the record as it was
/// deleted, or as a soft destroy stored it.
pub async fn destroy_dynamic_by_id<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    expected_version: Option<i64>,
) -> Result<FieldMap> {
    destroy_dynamic_via(ctx, resource, None, action, id, FieldMap::new(), expected_version).await
}

/// [`destroy_dynamic_by_id`] through the read `read` (by default the one `action`
/// upgrades with, else the primary read), which decides which records it finds, as an
/// RPC action's `read_action` does; given `input`, the action's arguments and attributes
/// a soft destroy writes.
pub async fn destroy_dynamic_via<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    read: Option<&'static ActionDef>,
    action: &'static ActionDef,
    id: Uuid,
    input: FieldMap,
    expected_version: Option<i64>,
) -> Result<FieldMap> {
    expect_kind(action, ActionKind::Destroy)?;
    let read = match read {
        Some(read) => Some(read),
        None => super::atomic::finding_read(resource, action)?,
    };
    let scope = super::atomic::read_scope(resource, read, ctx.actor.as_ref())?;
    let atomic = if action.soft {
        ctx.data.can_update_atomically(resource)
    } else {
        ctx.data.can_destroy_atomically(resource)
    };
    if atomic {
        let planned = crate::pipeline::split_input(action, input.clone()).and_then(|(accepted, arguments)| {
            let plan = super::atomic::plan_update(
                resource,
                action,
                super::atomic::PlanInput {
                    actor: ctx.actor.as_ref(),
                    tenant: ctx.tenant(),
                    sets: accepted,
                    arguments: &arguments,
                    expected_version: expected_version.map(|version| (id, version)),
                    collect_hooks: true,
                },
            )?;
            Ok((plan, arguments))
        });
        match found_first(ctx, resource, id, &scope, planned).await? {
            (Ok(plan), arguments) => {
                return DynamicChangeset::commit_atomic_by_id(ctx, resource, action, id, arguments, plan, scope).await;
            }
            // Only a soft destroy must be atomic, as in Ash: a hard one reads first.
            (Err(reason), _) if action.soft && action.require_atomic => {
                return Err(Error::MustBeAtomic {
                    resource: resource.name,
                    action: action.name,
                    reason,
                });
            }
            (Err(_), _) => {}
        }
    }
    let existing = read_visible(ctx, resource, id, scope).await?;
    if let (Some(expected), Some(version)) = (expected_version, resource.optimistic_lock_attribute())
        && existing.get(version).and_then(Value::as_int).unwrap_or(1) != expected
    {
        return Err(Error::StaleRecord { resource: resource.name, id });
    }
    let cascade = super::managed::Cascade::new(true);
    destroy_dynamic_input(ctx, resource, action, id, &existing, input, &cascade).await
}

/// A plan for an update or destroy by id, unless its policies forbid it whatever the
/// record: then the record must be found first, as Ash finds it before the change's
/// policies refuse it (they're part of its statement), so that one the actor can't read
/// is [`Error::NotFound`] even where the change is forbidden too. Invalid input fails
/// first, as it does in Ash.
async fn found_first<D: DataLayer, T>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    id: Uuid,
    scope: &Option<Filter>,
    planned: Result<T>,
) -> Result<T> {
    if matches!(planned, Err(Error::Forbidden)) {
        read_visible(ctx, resource, id, scope.clone()).await?;
    }
    planned
}

/// Record `id` as a read in `ctx` would see it: not another tenant's, nor one outside
/// `scope`, the read that finds it; or, given none, one its primary read hides.
async fn read_visible<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    id: Uuid,
    scope: Option<Filter>,
) -> Result<FieldMap> {
    let pk = pk_name(resource)?;
    let by_id = Some(Filter::eq(pk, Value::Uuid(id)));
    let (filter, tenant) = match scope {
        Some(scope) => {
            let (filter, tenant) = crate::pipeline::apply_tenant_scope(resource, by_id, ctx.tenant.clone())?;
            (crate::pipeline::and_filters(filter, Some(scope)), tenant)
        }
        None => crate::pipeline::visible_scope(resource, by_id, ctx.tenant.clone())?,
    };
    ctx.data
        .run_query(resource, &CompiledQuery { filter, tenant, actor: ctx.actor.clone(), ..CompiledQuery::default() })
        .await?
        .into_iter()
        .next()
        .ok_or(Error::NotFound)
}

/// Runs a create through `action` on any resource, as a typed create would.
pub async fn create_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    input: FieldMap,
) -> Result<FieldMap> {
    DynamicChangeset::for_create(ctx, resource, action, input)?
        .commit(ctx)
        .await
}

/// Runs an update through `action` of the record `id` as the context reads it, as a
/// typed update would.
pub async fn update_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    input: FieldMap,
) -> Result<FieldMap> {
    update_dynamic_expecting(ctx, resource, action, id, input, None).await
}

/// [`update_dynamic`] of a record that must still have lock version `expected_version`,
/// else [`Error::StaleRecord`]. As in Ash, it runs as one statement when the data layer
/// can and the action allows: the update by id, with its validations, policies and the
/// version checked in it, and no read first. Otherwise it reads the record, as the
/// context sees it, and updates that. Either way it finds the record as AshGraphql and
/// AshTypescript do, through the read action, whose policies the actor must pass: one it
/// can't read is [`Error::NotFound`].
pub async fn update_dynamic_expecting<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    id: Uuid,
    input: FieldMap,
    expected_version: Option<i64>,
) -> Result<FieldMap> {
    update_dynamic_via(ctx, resource, None, action, id, input, expected_version).await
}

/// [`update_dynamic_expecting`] through the read `read` (by default the one `action`
/// upgrades with, else the primary read), which decides which records it finds, as an
/// RPC action's `read_action` does.
pub async fn update_dynamic_via<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    read: Option<&'static ActionDef>,
    action: &'static ActionDef,
    id: Uuid,
    input: FieldMap,
    expected_version: Option<i64>,
) -> Result<FieldMap> {
    expect_kind(action, ActionKind::Update)?;
    let read = match read {
        Some(read) => Some(read),
        None => super::atomic::finding_read(resource, action)?,
    };
    let scope = super::atomic::read_scope(resource, read, ctx.actor.as_ref())?;
    if ctx.data.can_update_atomically(resource) {
        let planned = crate::pipeline::split_input(action, input.clone()).and_then(|(accepted, arguments)| {
            let plan = super::atomic::plan_update(
                resource,
                action,
                super::atomic::PlanInput {
                    actor: ctx.actor.as_ref(),
                    tenant: ctx.tenant(),
                    sets: accepted,
                    arguments: &arguments,
                    expected_version: expected_version.map(|version| (id, version)),
                    collect_hooks: true,
                },
            )?;
            Ok((plan, arguments))
        });
        match found_first(ctx, resource, id, &scope, planned).await? {
            (Ok(plan), arguments) => {
                return DynamicChangeset::commit_atomic_by_id(ctx, resource, action, id, arguments, plan, scope).await;
            }
            (Err(reason), _) if action.require_atomic => {
                return Err(Error::MustBeAtomic {
                    resource: resource.name,
                    action: action.name,
                    reason,
                });
            }
            (Err(_), _) => {}
        }
    }
    let existing = read_visible(ctx, resource, id, scope).await?;
    if let (Some(expected), Some(version)) = (expected_version, resource.optimistic_lock_attribute())
        && existing.get(version).and_then(Value::as_int).unwrap_or(1) != expected
    {
        return Err(Error::StaleRecord { resource: resource.name, id });
    }
    update_existing_dynamic(ctx, resource, action, existing, input).await
}

/// [`update_dynamic`] of a record already read, as `ctx` sees it: no second read.
pub async fn update_existing_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    existing: FieldMap,
    input: FieldMap,
) -> Result<FieldMap> {
    expect_kind(action, ActionKind::Update)?;
    DynamicChangeset::for_update(ctx, resource, action, existing, input)?
        .commit(ctx)
        .await
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
    let stored = ctx.data.create(&R::DEF, ctx.tenant.as_deref(), id, fields).await?;
    R::from_fields(&stored)
}
