use std::collections::{HashMap, HashSet};
use uuid::Uuid;

use crate::action::{ActionKind, ManagedRelType};
use crate::changeset::ManagedRelationshipSpec;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{generate_pk, pk_name};
use crate::resource::{OnDelete, RelKind, ResourceDef};
use crate::value::{FieldMap, Value, required_uuid};

use super::lifecycle::{create_dynamic, destroy_dynamic, update_dynamic};

pub async fn handle_cascading_deletes<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    parent_fields: &FieldMap,
) -> Result<()> {
    for rel in resource.relationships {
        match rel.kind {
            RelKind::HasMany => {
                match rel.on_delete {
                    OnDelete::Nothing => {}
                    OnDelete::Restrict => {
                        let dest_def = (rel.destination)();
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
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
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
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
                        let child_destroy_action = dest_def
                            .actions
                            .iter()
                            .find(|a| a.kind == ActionKind::Destroy && a.primary)
                            .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                        for child_row in rows {
                            let child_id = required_uuid(&child_row, child_pk)?;
                            if let Some(destroy_act) = child_destroy_action {
                                Box::pin(destroy_dynamic(
                                    ctx,
                                    dest_def,
                                    destroy_act,
                                    child_id,
                                    &child_row,
                                ))
                                .await?;
                            } else {
                                ctx.data.destroy(dest_def, child_id).await?;
                            }
                        }
                    }
                    OnDelete::Nilify => {
                        let dest_def = (rel.destination)();
                        let parent_val = parent_fields
                            .get(rel.source_attribute)
                            .cloned()
                            .unwrap_or_else(|| Value::from(parent_id));
                        let filter = Filter::eq(rel.destination_attribute, parent_val);
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
                            patch.insert(rel.destination_attribute.to_string(), Value::Null);
                            ctx.data.update(dest_def, child_id, patch).await?;
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
                            let join_destroy_action = through_def
                                .actions
                                .iter()
                                .find(|a| a.kind == ActionKind::Destroy && a.primary)
                                .or_else(|| through_def.actions.iter().find(|a| a.kind == ActionKind::Destroy));

                            for join_row in rows {
                                let join_id = required_uuid(&join_row, join_pk)?;
                                if let Some(destroy_act) = join_destroy_action {
                                    Box::pin(destroy_dynamic(
                                        ctx,
                                        through_def,
                                        destroy_act,
                                        join_id,
                                        &join_row,
                                    ))
                                    .await?;
                                } else {
                                    ctx.data.destroy(through_def, join_id).await?;
                                }
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

pub async fn handle_managed_relationships<D: DataLayer>(
    ctx: &Context<D>,
    resource: &'static ResourceDef,
    parent_id: Uuid,
    managed_list: Vec<ManagedRelationshipSpec>,
) -> Result<()> {
    for managed in managed_list {
        let rel = resource
            .relationship(managed.relationship)
            .ok_or_else(|| Error::Invalid(format!("unknown relationship `{}` on {}", managed.relationship, resource.name)))?;

        match rel.kind {
            RelKind::HasMany => {
                let dest_def = (rel.destination)();
                let child_fk = rel.destination_attribute;
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
                            child_fields.insert(child_fk.to_string(), Value::from(parent_id));
                            if let Some(create_act) = child_create_action {
                                Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                            } else {
                                generate_pk(dest_def, &mut child_fields);
                                let child_id = required_uuid(&child_fields, child_pk)?;
                                ctx.data.create(dest_def, child_id, child_fields).await?;
                            }
                        }
                    }
                    ManagedRelType::DirectControl => {
                        let filter = Filter::eq(child_fk, Value::from(parent_id));
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
                            child_fields.insert(child_fk.to_string(), Value::from(parent_id));
                            let given_id = child_fields
                                .get(child_pk)
                                .and_then(|v| match v {
                                    Value::Uuid(u) => Some(*u),
                                    Value::String(s) => Uuid::parse_str(s).ok(),
                                    _ => None,
                                });

                            if let Some(child_id) = given_id && existing_map.contains_key(&child_id) {
                                kept_ids.insert(child_id);
                                if let Some(update_act) = child_update_action {
                                    Box::pin(update_dynamic(ctx, dest_def, update_act, child_id, child_fields)).await?;
                                } else {
                                    ctx.data.update(dest_def, child_id, child_fields).await?;
                                }
                            } else {
                                let new_id = if let Some(create_act) = child_create_action {
                                    let stored = Box::pin(create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                                    required_uuid(&stored, child_pk)?
                                } else {
                                    generate_pk(dest_def, &mut child_fields);
                                    let child_id = required_uuid(&child_fields, child_pk)?;
                                    ctx.data.create(dest_def, child_id, child_fields).await?;
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
                                        let mut patch = FieldMap::new();
                                        patch.insert(child_fk.to_string(), Value::Null);
                                        Box::pin(update_dynamic(ctx, dest_def, update_act, existing_id, patch)).await?;
                                    } else {
                                        let mut updated = existing_fields;
                                        updated.insert(child_fk.to_string(), Value::Null);
                                        ctx.data.update(dest_def, existing_id, updated).await?;
                                    }
                                } else if let Some(destroy_act) = child_destroy_action {
                                    Box::pin(destroy_dynamic(ctx, dest_def, destroy_act, existing_id, &existing_fields)).await?;
                                } else {
                                    ctx.data.destroy(dest_def, existing_id).await?;
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
                                        ctx.data.create(dest_def, child_id, child_fields).await?;
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
                                    ctx.data.create(through_def, join_id, join_fields).await?;
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
                                        ctx.data.destroy(through_def, join_id).await?;
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
                                        ctx.data.create(dest_def, child_id, child_fields).await?;
                                        child_id
                                    }
                                };

                                let mut join_fields = FieldMap::new();
                                join_fields.insert(source_fk.to_string(), Value::from(parent_id));
                                join_fields.insert(dest_fk.to_string(), Value::from(target_id));
                                generate_pk(through_def, &mut join_fields);
                                let join_id = required_uuid(&join_fields, join_pk)?;
                                ctx.data.create(through_def, join_id, join_fields).await?;
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
