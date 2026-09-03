use std::future::Future;
use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, PersistKind};
use crate::changeset::{self, Changeset};
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{
    action_named, expect_kind, expect_persist, generate_pk, pk_name,
    run_validations_with_context, validate,
};
use crate::policy::authorize_write;
use crate::resource::{Resource, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

use super::managed::handle_cascading_deletes;
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

    crate::pipeline::apply_changes_with_context(
        &mut fields,
        action,
        ctx.actor.as_ref(),
        ctx.tenant(),
        ctx.metadata(),
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
        Some(existing_fields),
    )?;

    let execute_destroy = || async {
        handle_cascading_deletes(ctx, resource, id, existing_fields).await?;
        ctx.data.destroy(resource, id).await?;

        for hook in dynamic_after_actions {
            let mut stored = existing_fields.clone();
            hook(&mut stored)?;
        }

        let notification = crate::notifier::Notification::new(
            resource.name,
            action.name,
            ActionKind::Destroy,
            id,
            existing_fields.clone(),
            Some(existing_fields.clone()),
            ctx.actor.clone(),
            ctx.metadata.clone(),
        ).with_tenant(ctx.tenant.clone());
        crate::notifier::dispatch_notification(ctx, resource, notification).await?;

        Ok(())
    };

    match execute_destroy().await {
        Ok(()) => {
            for hook in dynamic_after_transactions {
                hook(Ok(existing_fields));
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

pub async fn create_dynamic<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    mut fields: FieldMap,
) -> Result<FieldMap> {
    generate_pk(resource, &mut fields);
    let id = required_uuid(&fields, pk_name(resource)?)?;

    if let Some(v_attr) = resource.optimistic_lock_attribute() {
        fields.insert(v_attr.to_string(), Value::Int(1));
    }

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
    run_validations_with_context(
        resource,
        action,
        None,
        &fields,
        ctx.actor.as_ref(),
        ctx.tenant(),
        ctx.metadata(),
        &FieldMap::new(),
    )?;
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
        ctx.metadata.clone(),
    ).with_tenant(ctx.tenant.clone());
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
                tenant: ctx.tenant.clone(),
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
    run_validations_with_context(
        resource,
        action,
        Some(&existing_fields),
        &fields,
        ctx.actor.as_ref(),
        ctx.tenant(),
        ctx.metadata(),
        &FieldMap::new(),
    )?;
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
        ctx.metadata.clone(),
    ).with_tenant(ctx.tenant.clone());
    crate::notifier::dispatch_notification(ctx, resource, notification).await?;

    Ok(stored)
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
