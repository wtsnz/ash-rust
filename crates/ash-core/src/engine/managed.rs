use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, ManagedRelType};
use crate::changeset::ManagedRelationshipSpec;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{generate_pk, pk_name};
use crate::resource::{OnDelete, RelKind, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

use super::lifecycle::{create_dynamic, destroy_dynamic, destroy_dynamic_with};
use crate::changeset::DynamicChangeset;

/// State shared by one destroy and everything it cascades into.
pub(crate) struct Cascade {
    /// Records being destroyed further up, so a cycle in the data ends instead of recursing.
    visited: std::sync::Mutex<HashSet<(&'static str, Uuid)>>,
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
    pub(crate) fn enter(&self, resource: &'static ResourceDef, id: Uuid) -> bool {
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
    child_id: Uuid,
    child_row: &FieldMap,
    cascade: &Cascade,
) -> Result<()> {
    match child_destroy_action(dest_def, hard) {
        Some(action) => {
            Box::pin(destroy_dynamic_with(ctx, dest_def, action, child_id, child_row, cascade))
                .await?;
        }
        None => {
            if cascade.enter(dest_def, child_id) {
                Box::pin(cascade_deletes(ctx, dest_def, child_id, child_row, cascade)).await?;
                ctx.data.destroy(dest_def, ctx.tenant.as_deref(), child_id).await?;
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
    parent_id: Uuid,
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
        if dest_def.actions.iter().all(|a| a.kind != ActionKind::Destroy) {
            return Err(Error::Invalid(format!(
                "cascade_destroy needs a destroy action on {}",
                dest_def.name
            )));
        }
        let Some(related) = rel.destination_filter(&parent) else {
            continue;
        };
        let read_filter = if hard { None } else { dest_def.primary_read_filter() };
        let filter = Filter::and([Some(related), read_filter].into_iter().flatten());
        let rows = ctx
            .data
            .run_query(
                dest_def,
                &CompiledQuery {
                    filter: Some(filter),
                    tenant: ctx.tenant.clone(),
                    ..CompiledQuery::default()
                },
            )
            .await?;
        let child_pk = pk_name(dest_def)?;
        for child_row in rows {
            let child_id = required_uuid(&child_row, child_pk)?;
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
    id: Uuid,
    existing_fields: &FieldMap,
    fields: FieldMap,
    cascade: &Cascade,
) -> Result<FieldMap> {
    if !cascade.enter(resource, id) {
        return Ok(existing_fields.clone());
    }
    if action.soft {
        let changes = soft_destroy_changes(resource, existing_fields, fields);
        let stored = ctx.data.update(resource, ctx.tenant.as_deref(), id, changes).await?;
        cascade_destroy_related(ctx, resource, action, id, existing_fields, cascade).await?;
        return Ok(stored);
    }
    cascade_destroy_related(ctx, resource, action, id, existing_fields, cascade).await?;
    cascade_deletes(ctx, resource, id, existing_fields, cascade).await?;
    ctx.data.destroy(resource, ctx.tenant.as_deref(), id).await?;
    Ok(existing_fields.clone())
}

/// What a soft destroy writes: the fields its changes set, a raised lock version, and a
/// new `updated_at`, like an update. Writing back the whole record would put nulls over
/// fields the actor was not allowed to read.
fn soft_destroy_changes(
    resource: &ResourceDef,
    existing_fields: &FieldMap,
    fields: FieldMap,
) -> FieldMap {
    let mut changes: FieldMap = fields
        .into_iter()
        .filter(|(name, value)| existing_fields.get(name) != Some(value))
        .collect();
    crate::pipeline::prepare_update_fields(resource, existing_fields, &mut changes);
    changes
}

/// `fields` with the primary key filled in, so a key built from it never misses `id`.
fn with_primary_key(resource: &ResourceDef, id: Uuid, fields: &FieldMap) -> Result<FieldMap> {
    let mut fields = fields.clone();
    fields
        .entry(pk_name(resource)?.to_string())
        .or_insert(Value::Uuid(id));
    Ok(fields)
}

/// Applies each relationship's `on_delete` before the record is deleted.
pub async fn handle_cascading_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    parent_fields: &FieldMap,
) -> Result<()> {
    let cascade = Cascade::new(true);
    cascade.enter(resource, parent_id);
    cascade_deletes(ctx, resource, parent_id, parent_fields, &cascade).await
}

/// [`handle_cascading_deletes`] within a destroy that is already under way. The parent is
/// hard-deleted, so children go with it even when their own destroy only archives.
pub(crate) async fn cascade_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    parent_fields: &FieldMap,
    cascade: &Cascade,
) -> Result<()> {
    let parent = with_primary_key(resource, parent_id, parent_fields)?;
    for rel in resource.relationships {
        match rel.kind {
            RelKind::HasMany | RelKind::HasOne => {
                match rel.on_delete {
                    OnDelete::Nothing => {}
                    OnDelete::Restrict => {
                        let dest_def = (rel.destination)();
                        let Some(filter) = rel.destination_filter(&parent) else {
                            continue;
                        };
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    tenant: ctx.tenant.clone(),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
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
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    tenant: ctx.tenant.clone(),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
                        let child_pk = pk_name(dest_def)?;
                        for child_row in rows {
                            let child_id = required_uuid(&child_row, child_pk)?;
                            destroy_child(ctx, dest_def, true, child_id, &child_row, cascade)
                                .await?;
                        }
                    }
                    OnDelete::Nilify => {
                        let dest_def = (rel.destination)();
                        let Some(filter) = rel.destination_filter(&parent) else {
                            continue;
                        };
                        let rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    tenant: ctx.tenant.clone(),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;
                        let child_pk = pk_name(dest_def)?;
                        for child_row in rows {
                            let child_id = required_uuid(&child_row, child_pk)?;
                            let mut patch = FieldMap::new();
                            for column in rel.destination_columns() {
                                patch.insert(column.to_string(), Value::Null);
                            }
                            ctx.data.update(dest_def, ctx.tenant.as_deref(), child_id, patch).await?;
                        }
                    }
                }
            }
            RelKind::ManyToMany => {
                if let Some(through_fn) = rel.through {
                    let through_def = through_fn();
                    let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                    let parent_val = parent_fields
                        .get(rel.source_attribute)
                        .cloned()
                        .unwrap_or_else(|| Value::from(parent_id));
                    let filter = Filter::eq(source_fk, parent_val);
                    match rel.on_delete {
                        OnDelete::Nothing => {}
                        OnDelete::Restrict => {
                            let rows = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(filter),
                                        tenant: ctx.tenant.clone(),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;
                            if !rows.is_empty() {
                                return Err(Error::DeleteRestricted {
                                    resource: resource.name,
                                    relationship: rel.name,
                                    count: rows.len(),
                                });
                            }
                        }
                        OnDelete::Cascade => {
                            let rows = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(filter),
                                        tenant: ctx.tenant.clone(),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;
                            let join_pk = pk_name(through_def)?;
                            for join_row in rows {
                                let join_id = required_uuid(&join_row, join_pk)?;
                                destroy_child(ctx, through_def, true, join_id, &join_row, cascade)
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
    Box::pin(DynamicChangeset::for_create_forcing(ctx, dest_def, action, input, forced)?.commit(ctx))
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
            .commit(ctx),
    )
    .await
}

pub async fn handle_managed_relationships<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    parent_fields: &FieldMap,
    managed_list: Vec<ManagedRelationshipSpec>,
) -> Result<()> {
    let parent = with_primary_key(resource, parent_id, parent_fields)?;
    for managed in managed_list {
        let rel = resource
            .relationship(managed.relationship)
            .ok_or_else(|| Error::Invalid(format!("unknown relationship `{}` on {}", managed.relationship, resource.name)))?;

        match rel.kind {
            RelKind::HasMany | RelKind::HasOne => {
                let dest_def = (rel.destination)();
                // Children take every key column from the parent.
                let link: Vec<(String, Value)> = rel
                    .key_pairs()
                    .into_iter()
                    .map(|(source, destination)| {
                        let value = parent.get(source).cloned().unwrap_or(Value::Null);
                        (destination.to_string(), value)
                    })
                    .collect();
                let child_pk = pk_name(dest_def)?;
                let child_create_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Create && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                let child_update_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Update && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Update));
                let child_destroy_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Destroy && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                match managed.rel_type {
                    ManagedRelType::Create | ManagedRelType::Append => {
                        for mut child_fields in managed.inputs {
                            if let Some(create_act) = child_create_action {
                                create_child(ctx, dest_def, create_act, child_fields, &link).await?;
                            } else {
                                child_fields.extend(link.iter().cloned());
                                generate_pk(dest_def, &mut child_fields);
                                let child_id = required_uuid(&child_fields, child_pk)?;
                                ctx.data.create(dest_def, ctx.tenant.as_deref(), child_id, child_fields).await?;
                            }
                        }
                    }
                    ManagedRelType::DirectControl => {
                        let filter = rel.destination_filter(&parent).unwrap_or(Filter::False);
                        let existing_rows = ctx
                            .data
                            .run_query(
                                dest_def,
                                &CompiledQuery {
                                    filter: Some(filter),
                                    tenant: ctx.tenant.clone(),
                                    ..CompiledQuery::default()
                                },
                            )
                            .await?;

                        let mut existing_map = HashMap::new();
                        for row in existing_rows {
                            if let Ok(id) = required_uuid(&row, child_pk) {
                                existing_map.insert(id, row);
                            }
                        }

                        let mut kept_ids = HashSet::new();
                        for mut child_fields in managed.inputs {
                            let given_id = child_fields
                                .get(child_pk)
                                .and_then(|v| match v {
                                    Value::Uuid(u) => Some(*u),
                                    Value::String(s) => Uuid::parse_str(s).ok(),
                                    _ => None,
                                });

                            if let Some(child_id) = given_id
                                && let Some(existing) = existing_map.get(&child_id)
                            {
                                kept_ids.insert(child_id);
                                if let Some(update_act) = child_update_action {
                                    child_fields.remove(child_pk);
                                    update_child(ctx, dest_def, update_act, existing.clone(), child_fields, &link)
                                        .await?;
                                } else {
                                    child_fields.extend(link.iter().cloned());
                                    ctx.data.update(dest_def, ctx.tenant.as_deref(), child_id, child_fields).await?;
                                }
                            } else {
                                // An id that names no child of this parent doesn't pick the new one's.
                                child_fields.remove(child_pk);
                                let new_id = if let Some(create_act) = child_create_action {
                                    let stored =
                                        create_child(ctx, dest_def, create_act, child_fields, &link).await?;
                                    required_uuid(&stored, child_pk)?
                                } else {
                                    child_fields.extend(link.iter().cloned());
                                    generate_pk(dest_def, &mut child_fields);
                                    let child_id = required_uuid(&child_fields, child_pk)?;
                                    ctx.data.create(dest_def, ctx.tenant.as_deref(), child_id, child_fields).await?;
                                    child_id
                                };
                                kept_ids.insert(new_id);
                            }
                        }

                        // Remove omitted children
                        for (existing_id, existing_fields) in existing_map {
                            if !kept_ids.contains(&existing_id) {
                                if rel.on_delete == OnDelete::Nilify {
                                    if let Some(update_act) = child_update_action {
                                        let unlink: Vec<(String, Value)> = link
                                            .iter()
                                            .map(|(column, _)| (column.clone(), Value::Null))
                                            .collect();
                                        update_child(ctx, dest_def, update_act, existing_fields, FieldMap::new(), &unlink)
                                            .await?;
                                    } else {
                                        let mut updated = existing_fields;
                                        for (column, _) in &link {
                                            updated.insert(column.clone(), Value::Null);
                                        }
                                        ctx.data.update(dest_def, ctx.tenant.as_deref(), existing_id, updated).await?;
                                    }
                                } else if let Some(destroy_act) = child_destroy_action {
                                    Box::pin(destroy_dynamic(ctx, dest_def, destroy_act, existing_id, &existing_fields)).await?;
                                } else {
                                    ctx.data.destroy(dest_def, ctx.tenant.as_deref(), existing_id).await?;
                                }
                            }
                        }
                    }
                }
            }
            RelKind::ManyToMany => {
                if let Some(through_fn) = rel.through {
                    let through_def = through_fn();
                    let dest_def = (rel.destination)();
                    let source_fk = rel.source_attribute_on_join_resource.unwrap_or("source_id");
                    let dest_fk = rel.destination_attribute_on_join_resource.unwrap_or("dest_id");
                    let join_pk = pk_name(through_def)?;
                    let dest_pk = pk_name(dest_def)?;

                    match managed.rel_type {
                        ManagedRelType::DirectControl => {
                            let join_filter = Filter::eq(source_fk, Value::from(parent_id));
                            let existing_joins = ctx
                                .data
                                .run_query(
                                    through_def,
                                    &CompiledQuery {
                                        filter: Some(join_filter),
                                        tenant: ctx.tenant.clone(),
                                        ..CompiledQuery::default()
                                    },
                                )
                                .await?;

                            let mut existing_join_map = HashMap::new();
                            for j_row in existing_joins {
                                if let Some(target_val) = j_row.get(dest_fk)
                                    && let Ok(join_id) = required_uuid(&j_row, join_pk)
                                    && let Some(target_uuid) = match target_val {
                                        Value::Uuid(u) => Some(*u),
                                        Value::String(s) => Uuid::parse_str(s).ok(),
                                        _ => None,
                                    }
                                {
                                    existing_join_map.insert(target_uuid, (join_id, j_row));
                                }
                            }

                            let mut kept_targets = HashSet::new();
                            for mut child_fields in managed.inputs {
                                let target_id = if let Ok(id) = required_uuid(&child_fields, dest_pk) {
                                    id
                                } else {
                                    let create_act = dest_def
                                        .actions
                                        .iter()
                                        .find(|a| a.kind == ActionKind::Create && a.primary)
                                        .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                                    if let Some(create_act) = create_act {
                                        let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                        required_uuid(&stored, dest_pk)?
                                    } else {
                                        generate_pk(dest_def, &mut child_fields);
                                        let child_id = required_uuid(&child_fields, dest_pk)?;
                                        ctx.data.create(dest_def, ctx.tenant.as_deref(), child_id, child_fields).await?;
                                        child_id
                                    }
                                };
                                kept_targets.insert(target_id);

                                if !existing_join_map.contains_key(&target_id) {
                                    let mut join_fields = FieldMap::new();
                                    join_fields.insert(source_fk.to_string(), Value::from(parent_id));
                                    join_fields.insert(dest_fk.to_string(), Value::from(target_id));
                                    generate_pk(through_def, &mut join_fields);
                                    let join_id = required_uuid(&join_fields, join_pk)?;
                                    ctx.data.create(through_def, ctx.tenant.as_deref(), join_id, join_fields).await?;
                                }
                            }

                            // Destroy join rows for omitted targets
                            let join_destroy_action = through_def
                                .actions
                                .iter()
                                .find(|a| a.kind == ActionKind::Destroy && a.primary)
                                .or_else(|| through_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                            for (target_id, (join_id, join_row)) in existing_join_map {
                                if !kept_targets.contains(&target_id) {
                                    if let Some(destroy_act) = join_destroy_action {
                                        Box::pin(destroy_dynamic(ctx, through_def, destroy_act, join_id, &join_row)).await?;
                                    } else {
                                        ctx.data.destroy(through_def, ctx.tenant.as_deref(), join_id).await?;
                                    }
                                }
                            }
                        }
                        ManagedRelType::Create | ManagedRelType::Append => {
                            for mut child_fields in managed.inputs {
                                let target_id = if let Ok(id) = required_uuid(&child_fields, dest_pk) {
                                    id
                                } else {
                                    let create_act = dest_def
                                        .actions
                                        .iter()
                                        .find(|a| a.kind == ActionKind::Create && a.primary)
                                        .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                                    if let Some(create_act) = create_act {
                                        let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                        required_uuid(&stored, dest_pk)?
                                    } else {
                                        generate_pk(dest_def, &mut child_fields);
                                        let child_id = required_uuid(&child_fields, dest_pk)?;
                                        ctx.data.create(dest_def, ctx.tenant.as_deref(), child_id, child_fields).await?;
                                        child_id
                                    }
                                };

                                let mut join_fields = FieldMap::new();
                                join_fields.insert(source_fk.to_string(), Value::from(parent_id));
                                join_fields.insert(dest_fk.to_string(), Value::from(target_id));
                                generate_pk(through_def, &mut join_fields);
                                let join_id = required_uuid(&join_fields, join_pk)?;
                                ctx.data.create(through_def, ctx.tenant.as_deref(), join_id, join_fields).await?;
                            }
                        }
                    }
                }
            }
            RelKind::BelongsTo => {}
        }
    }
    Ok(())
}
