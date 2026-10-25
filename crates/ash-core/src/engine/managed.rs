use std::collections::{HashMap, HashSet};

use crate::action::{ActionDef, ActionKind, ManagedRelType};
use crate::changeset::ManagedRelationshipSpec;
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::pk_name;
use crate::resource::{OnDelete, RelKind, ResourceDef};
use crate::value::{FieldMap, Value, required_pk};

use super::lifecycle::{destroy_dynamic, destroy_dynamic_with};
use crate::changeset::DynamicChangeset;
use crate::changeset::primary_action;

/// State shared by one destroy and everything it cascades into.
pub(crate) struct Cascade {
    /// Records being destroyed further up, so a cycle in the data ends instead of recursing.
    visited: std::sync::Mutex<HashSet<(&'static str, Value)>>,
    /// Whether cascaded destroys send notifications, as the top-level destroy does.
    pub(crate) notify: bool,
}

impl Cascade {
    pub(crate) fn new(notify: bool) -> Self {
        Self {
            visited: std::sync::Mutex::new(HashSet::new()),
            notify,
        }
    }

    /// Marks the record as being destroyed, returning false if it already is.
    pub(crate) fn enter(&self, resource: &'static ResourceDef, id: Value) -> bool {
        self.visited
            .lock()
            .map(|mut visited| visited.insert((resource.name, id)))
            .unwrap_or(true)
    }
}

/// The destroy action a cascade runs on a child. A hard delete of the parent removes the
/// row it references, so the child must go too: its archiving primary destroy would leave
/// it behind, pointing at nothing.
fn child_destroy_action(dest_def: &'static ResourceDef, hard: bool) -> Option<&'static ActionDef> {
    let destroys = || {
        dest_def
            .actions
            .iter()
            .filter(|a| a.kind == ActionKind::Destroy && !(hard && a.soft))
    };
    destroys().find(|a| a.primary).or_else(|| destroys().next())
}

/// Destroys one cascaded child with [`child_destroy_action`], or when a hard delete finds
/// no hard destroy action, deletes the row the way `ON DELETE CASCADE` would.
async fn destroy_child<D: DataLayer>(
    ctx: &Context<D>,
    dest_def: &'static ResourceDef,
    hard: bool,
    child_id: Value,
    child_row: &FieldMap,
    cascade: &Cascade,
) -> Result<()> {
    match child_destroy_action(dest_def, hard) {
        Some(action) => {
            Box::pin(destroy_dynamic_with(
                ctx, dest_def, action, child_id, child_row, cascade,
            ))
            .await?;
        }
        None => {
            static HARD_DESTROY: ActionDef = ActionDef::destroy("cascade_destroy");
            let action = primary_action(dest_def, ActionKind::Destroy).unwrap_or(&HARD_DESTROY);
            crate::policy::authorize_write(
                dest_def,
                action,
                ctx.authorization_actor(),
                Some(child_row),
            )?;
            if cascade.enter(dest_def, child_id.clone()) {
                Box::pin(cascade_deletes(
                    ctx,
                    dest_def,
                    child_id.clone(),
                    child_row,
                    cascade,
                ))
                .await?;
                super::lock::destroy_guarded(ctx, dest_def, child_id, None).await?;
            }
        }
    }
    Ok(())
}

/// Destroys the records related through each of `action.cascade_destroy`. A soft destroy
/// archives the children its primary read still shows; a hard one removes every child.
pub(crate) async fn cascade_destroy_related<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &ActionDef,
    parent_id: Value,
    parent_fields: &FieldMap,
    cascade: &Cascade,
) -> Result<()> {
    let parent = with_primary_key(resource, parent_id, parent_fields)?;
    let hard = !action.soft;
    for name in action.cascade_destroy {
        let rel = resource.relationship(name).ok_or_else(|| {
            Error::Invalid(format!(
                "cascade_destroy names unknown relationship `{name}` on {}",
                resource.name
            ))
        })?;
        if !matches!(rel.kind, RelKind::HasMany | RelKind::HasOne) {
            return Err(Error::Invalid(format!(
                "cascade_destroy supports has_many and has_one, but `{name}` on {} is not",
                resource.name
            )));
        }
        let dest_def = (rel.destination)();
        if dest_def
            .actions
            .iter()
            .all(|a| a.kind != ActionKind::Destroy)
        {
            return Err(Error::Invalid(format!(
                "cascade_destroy needs a destroy action on {}",
                dest_def.name
            )));
        }
        let Some(filter) = rel.destination_filter(&parent) else {
            continue;
        };
        let rows = ctx.access().related_rows(dest_def, filter, hard).await?;
        let child_pk = pk_name(dest_def)?;
        for child_row in rows {
            let child_id = required_pk(&child_row, child_pk)?;
            destroy_child(ctx, dest_def, hard, child_id, &child_row, cascade).await?;
        }
    }
    Ok(())
}

/// Removes one record for a destroy action after its changes ran. A hard destroy runs its
/// cascades and then deletes. A soft destroy updates the record first and archives its
/// children after, as AshArchival does, so a cycle stops at rows already hidden. Returns
/// what was stored.
pub(crate) async fn persist_destroy<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    action: &ActionDef,
    id: Value,
    existing_fields: &FieldMap,
    fields: FieldMap,
    cascade: &Cascade,
) -> Result<FieldMap> {
    if !cascade.enter(resource, id.clone()) {
        return Ok(existing_fields.clone());
    }
    let stored = if cascades(resource, action) {
        let stored = ctx
            .access()
            .related_rows(
                resource,
                Filter::eq(pk_name(resource)?, id.clone()),
                !action.soft,
            )
            .await?
            .into_iter()
            .next()
            .ok_or(Error::NotFound)?;
        crate::policy::authorize_write(resource, action, ctx.authorization_actor(), Some(&stored))?;
        stored
    } else {
        existing_fields.clone()
    };
    let lock = super::lock::lock_of(action, existing_fields);
    if action.soft {
        // The guarded write comes first, so a stale record archives nothing.
        let changes = soft_destroy_changes(resource, action, existing_fields, fields);
        let stored = super::lock::update_guarded(ctx, resource, id.clone(), changes, lock).await?;
        cascade_destroy_related(ctx, resource, action, id, &stored, cascade).await?;
        return Ok(stored);
    }
    // A stale record's related records stay: its lock is checked before they go.
    if let Some(lock) = &lock
        && cascades(resource, action)
    {
        super::lock::check_lock(ctx, resource, id.clone(), lock).await?;
    }
    cascade_destroy_related(ctx, resource, action, id.clone(), &stored, cascade).await?;
    cascade_deletes(ctx, resource, id.clone(), &stored, cascade).await?;
    super::lock::destroy_guarded(ctx, resource, id, lock.as_ref()).await?;
    Ok(stored)
}

/// Whether a hard destroy through `action` acts on related records before the record
/// goes: its `cascade_destroy`, or a relationship's `on_delete`.
pub(crate) fn cascades(resource: &ResourceDef, action: &ActionDef) -> bool {
    !action.cascade_destroy.is_empty()
        || (!action.soft
            && resource
                .relationships
                .iter()
                .any(|rel| rel.kind != RelKind::BelongsTo && rel.on_delete != OnDelete::Nothing))
}

/// What a soft destroy writes: the fields its changes set, a raised lock version, and a
/// new `updated_at`, like an update. Writing back the whole record would put nulls over
/// fields the actor was not allowed to read.
fn soft_destroy_changes(
    resource: &ResourceDef,
    action: &ActionDef,
    existing_fields: &FieldMap,
    fields: FieldMap,
) -> FieldMap {
    let mut changes: FieldMap = fields
        .into_iter()
        .filter(|(name, value)| existing_fields.get(name) != Some(value))
        .collect();
    crate::pipeline::prepare_update_fields(resource, action, existing_fields, &mut changes);
    changes
}

/// `fields` with the primary key filled in, so a key built from it never misses `id`.
fn with_primary_key(resource: &ResourceDef, id: Value, fields: &FieldMap) -> Result<FieldMap> {
    let mut fields = fields.clone();
    fields
        .entry(pk_name(resource)?.to_string())
        .or_insert(id.clone());
    Ok(fields)
}

/// Applies each relationship's `on_delete` before the record is deleted.
pub async fn handle_cascading_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Value,
    _parent_fields: &FieldMap,
) -> Result<()> {
    ctx.atomically(|tx| async move {
        let stored = tx
            .access()
            .related_rows(
                resource,
                Filter::eq(pk_name(resource)?, parent_id.clone()),
                true,
            )
            .await?
            .into_iter()
            .next()
            .ok_or(Error::NotFound)?;
        static DESTROY: ActionDef = ActionDef::destroy("cascade_destroy");
        let action = primary_action(resource, ActionKind::Destroy).unwrap_or(&DESTROY);
        crate::policy::authorize_write(resource, action, tx.authorization_actor(), Some(&stored))?;
        let cascade = Cascade::new(true);
        cascade.enter(resource, parent_id.clone());
        cascade_deletes(&tx, resource, parent_id, &stored, &cascade).await
    })
    .await
}

/// [`handle_cascading_deletes`] within a destroy that is already under way. The parent is
/// hard-deleted, so children go with it even when their own destroy only archives.
pub(crate) async fn cascade_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Value,
    parent_fields: &FieldMap,
    cascade: &Cascade,
) -> Result<()> {
    let parent = with_primary_key(resource, parent_id.clone(), parent_fields)?;
    for rel in resource.relationships {
        match rel.kind {
            RelKind::HasMany | RelKind::HasOne => match rel.on_delete {
                OnDelete::Nothing => {}
                OnDelete::Restrict => {
                    let dest_def = (rel.destination)();
                    let Some(filter) = rel.destination_filter(&parent) else {
                        continue;
                    };
                    let rows = ctx.access().related_rows(dest_def, filter, true).await?;
                    if !rows.is_empty() {
                        return Err(Error::DeleteRestricted {
                            resource: resource.name,
                            relationship: rel.name,
                            count: rows.len(),
                        });
                    }
                }
                OnDelete::Cascade => {
                    let dest_def = (rel.destination)();
                    let Some(filter) = rel.destination_filter(&parent) else {
                        continue;
                    };
                    let rows = ctx.access().related_rows(dest_def, filter, true).await?;
                    let child_pk = pk_name(dest_def)?;
                    for child_row in rows {
                        let child_id = required_pk(&child_row, child_pk)?;
                        destroy_child(ctx, dest_def, true, child_id, &child_row, cascade).await?;
                    }
                }
                OnDelete::Nilify => {
                    let dest_def = (rel.destination)();
                    let Some(filter) = rel.destination_filter(&parent) else {
                        continue;
                    };
                    let rows = ctx.access().related_rows(dest_def, filter, true).await?;
                    for child_row in rows {
                        let mut patch = FieldMap::new();
                        for column in rel.destination_columns() {
                            patch.insert(column.to_string(), Value::Null);
                        }
                        static NILIFY: ActionDef =
                            ActionDef::update("cascade_nilify").require_atomic(false);
                        let action =
                            primary_action(dest_def, ActionKind::Update).unwrap_or(&NILIFY);
                        update_child(
                            ctx,
                            dest_def,
                            action,
                            child_row,
                            FieldMap::new(),
                            &patch.into_iter().collect::<Vec<_>>(),
                        )
                        .await?;
                    }
                }
            },
            RelKind::ManyToMany => {
                if let Some(through_fn) = rel.through {
                    let through_def = through_fn();
                    let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                    let parent_val = parent_fields
                        .get(rel.source_attribute)
                        .cloned()
                        .unwrap_or_else(|| parent_id.clone());
                    let filter = Filter::eq(source_fk, parent_val);
                    match rel.on_delete {
                        OnDelete::Nothing => {}
                        OnDelete::Restrict => {
                            let rows = ctx.access().related_rows(through_def, filter, true).await?;
                            if !rows.is_empty() {
                                return Err(Error::DeleteRestricted {
                                    resource: resource.name,
                                    relationship: rel.name,
                                    count: rows.len(),
                                });
                            }
                        }
                        OnDelete::Cascade => {
                            let rows = ctx.access().related_rows(through_def, filter, true).await?;
                            let join_pk = pk_name(through_def)?;
                            for join_row in rows {
                                let join_id = required_pk(&join_row, join_pk)?;
                                let action =
                                    child_destroy_action(through_def, true).ok_or_else(|| {
                                        Error::Invalid(format!(
                                            "cascade needs a hard destroy action on {}",
                                            through_def.name
                                        ))
                                    })?;
                                Box::pin(destroy_dynamic_with(
                                    ctx,
                                    through_def,
                                    action,
                                    join_id,
                                    &join_row,
                                    cascade,
                                ))
                                .await?;
                            }
                        }
                        OnDelete::Nilify => {}
                    }
                }
            }
            RelKind::BelongsTo => {}
        }
    }
    Ok(())
}

/// Creates a managed child through its create action. The caller's input goes through
/// the action's accept list; the keys linking it to its parent are forced onto it, as
/// Ash's `force_change_attribute` does.
async fn create_child<D: DataLayer>(
    ctx: &Context<D>,
    dest_def: &'static ResourceDef,
    action: &'static ActionDef,
    input: FieldMap,
    link: &[(String, Value)],
) -> Result<FieldMap> {
    let forced = link.iter().cloned().collect();
    Box::pin(
        DynamicChangeset::for_create_forcing(ctx, dest_def, action, input, forced)?
            .commit_related(ctx),
    )
    .await
}

/// Updates the managed child `existing` through its update action, forcing `link` as
/// [`create_child`] does.
async fn update_child<D: DataLayer>(
    ctx: &Context<D>,
    dest_def: &'static ResourceDef,
    action: &'static ActionDef,
    existing: FieldMap,
    input: FieldMap,
    link: &[(String, Value)],
) -> Result<FieldMap> {
    let forced = link.iter().cloned().collect();
    Box::pin(
        DynamicChangeset::for_update_forcing(ctx, dest_def, action, existing, input, forced)?
            .commit_related(ctx),
    )
    .await
}

fn required_action(resource: &'static ResourceDef, kind: ActionKind) -> Result<&'static ActionDef> {
    primary_action(resource, kind).ok_or_else(|| {
        Error::Invalid(format!(
            "managed relationship needs a {} action on {}",
            kind.as_str(),
            resource.name,
        ))
    })
}

async fn target<D: DataLayer>(
    ctx: &Context<D>,
    dest: &'static ResourceDef,
    input: FieldMap,
) -> Result<FieldMap> {
    let pk = pk_name(dest)?;
    if input.contains_key(pk) {
        let id = required_pk(&input, pk).map_err(|_| Error::NotFound)?;
        ctx.access().resolve_related(dest, id).await
    } else {
        let action = required_action(dest, ActionKind::Create)?;
        create_child(ctx, dest, action, input, &[]).await
    }
}

pub async fn handle_managed_relationships<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Value,
    parent_fields: &FieldMap,
    managed_list: Vec<ManagedRelationshipSpec>,
) -> Result<()> {
    if managed_list.is_empty() {
        return Ok(());
    }
    ctx.atomically(|tx| async move {
        manage(&tx, resource, parent_id, parent_fields, managed_list).await
    })
    .await
}

async fn manage<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Value,
    parent_fields: &FieldMap,
    managed_list: Vec<ManagedRelationshipSpec>,
) -> Result<()> {
    let parent = with_primary_key(resource, parent_id, parent_fields)?;
    for managed in managed_list {
        let rel = resource.relationship(managed.relationship).ok_or_else(|| {
            Error::Invalid(format!(
                "unknown relationship `{}` on {}",
                managed.relationship, resource.name,
            ))
        })?;
        let dest = (rel.destination)();
        match rel.kind {
            RelKind::HasMany | RelKind::HasOne => {
                let link: Vec<_> = rel
                    .key_pairs()
                    .into_iter()
                    .map(|(source, destination)| {
                        (
                            destination.to_string(),
                            parent.get(source).cloned().unwrap_or(Value::Null),
                        )
                    })
                    .collect();
                let pk = pk_name(dest)?;
                let mut existing = HashMap::new();
                if managed.rel_type == ManagedRelType::DirectControl {
                    for row in ctx
                        .access()
                        .related_rows(
                            dest,
                            rel.destination_filter(&parent).unwrap_or(Filter::False),
                            false,
                        )
                        .await?
                    {
                        existing.insert(required_pk(&row, pk)?, row);
                    }
                }
                let mut kept = HashSet::new();
                for mut input in managed.inputs {
                    let id = input
                        .get(pk)
                        .and_then(|value| crate::value::pk_cast(dest.primary_key()?.ty, value));
                    if managed.rel_type == ManagedRelType::DirectControl
                        && let Some(id) = id
                        && let Some(row) = existing.get(&id)
                    {
                        kept.insert(id);
                        input.remove(pk);
                        update_child(
                            ctx,
                            dest,
                            required_action(dest, ActionKind::Update)?,
                            row.clone(),
                            input,
                            &link,
                        )
                        .await?;
                    } else {
                        // An unmatched DirectControl id must never select or become the new child.
                        if managed.rel_type == ManagedRelType::DirectControl {
                            input.remove(pk);
                        }
                        let row = create_child(
                            ctx,
                            dest,
                            required_action(dest, ActionKind::Create)?,
                            input,
                            &link,
                        )
                        .await?;
                        kept.insert(required_pk(&row, pk)?);
                    }
                }
                for (id, row) in existing {
                    if kept.contains(&id) {
                        continue;
                    }
                    if rel.on_delete == OnDelete::Nilify {
                        let unlink: Vec<_> = link
                            .iter()
                            .map(|(name, _)| (name.clone(), Value::Null))
                            .collect();
                        update_child(
                            ctx,
                            dest,
                            required_action(dest, ActionKind::Update)?,
                            row,
                            FieldMap::new(),
                            &unlink,
                        )
                        .await?;
                    } else {
                        Box::pin(destroy_dynamic(
                            ctx,
                            dest,
                            required_action(dest, ActionKind::Destroy)?,
                            id,
                            &row,
                        ))
                        .await?;
                    }
                }
            }
            RelKind::ManyToMany => {
                let through = rel
                    .through
                    .ok_or_else(|| Error::Invalid("many_to_many needs a join resource".into()))?(
                );
                let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                let dest_fk = rel
                    .destination_attribute_on_join_resource
                    .unwrap_or("dest_id");
                let source = parent
                    .get(rel.source_attribute)
                    .cloned()
                    .ok_or(Error::NotFound)?;
                let join_pk = pk_name(through)?;
                let mut existing = HashMap::new();
                if managed.rel_type == ManagedRelType::DirectControl {
                    for row in ctx
                        .access()
                        .related_rows(through, Filter::eq(source_fk, source.clone()), false)
                        .await?
                    {
                        let target = row.get(dest_fk).cloned().ok_or(Error::NotFound)?;
                        existing.insert(target, row);
                    }
                }
                let mut kept = HashSet::new();
                for input in managed.inputs {
                    let row = target(ctx, dest, input).await?;
                    let target = row
                        .get(rel.destination_attribute)
                        .cloned()
                        .ok_or(Error::NotFound)?;
                    kept.insert(target.clone());
                    if !existing.contains_key(&target) {
                        let link = [
                            (source_fk.to_string(), source.clone()),
                            (dest_fk.to_string(), target),
                        ];
                        create_child(
                            ctx,
                            through,
                            required_action(through, ActionKind::Create)?,
                            FieldMap::new(),
                            &link,
                        )
                        .await?;
                    }
                }
                for (target, row) in existing {
                    if !kept.contains(&target) {
                        let id = required_pk(&row, join_pk)?;
                        Box::pin(destroy_dynamic(
                            ctx,
                            through,
                            required_action(through, ActionKind::Destroy)?,
                            id,
                            &row,
                        ))
                        .await?;
                    }
                }
            }
            RelKind::BelongsTo => {}
        }
    }
    Ok(())
}
