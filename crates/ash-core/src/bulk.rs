use uuid::Uuid;

use crate::action::{ActionKind, PersistKind};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{
    action_named, apply_changes_with_context, expect_kind, expect_persist, generate_pk, pk_name,
    run_validations_with_context, split_input, validate,
};
use crate::policy::{authorize_field_writes, authorize_write, redact_fields};
use crate::resource::Resource;
use crate::value::{FieldMap, Value, required_uuid};

/// Options for configuring a bulk create operation.
#[derive(Clone, Debug)]
pub struct BulkCreateOptions {
    pub batch_size: Option<usize>,
    pub return_records: bool,
    pub stop_on_error: bool,
    pub notify: bool,
    pub upsert: Option<(&'static str, Vec<String>)>,
}

impl Default for BulkCreateOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
            upsert: None,
        }
    }
}

impl BulkCreateOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size);
        self
    }

    pub fn return_records(mut self, return_records: bool) -> Self {
        self.return_records = return_records;
        self
    }

    pub fn stop_on_error(mut self, stop: bool) -> Self {
        self.stop_on_error = stop;
        self
    }

    pub fn notify(mut self, notify: bool) -> Self {
        self.notify = notify;
        self
    }

    pub fn upsert(mut self, identity: &'static str, update_fields: &[&str]) -> Self {
        self.upsert = Some((
            identity,
            update_fields.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }
}

/// Options for configuring a bulk destroy operation.
#[derive(Clone, Debug)]
pub struct BulkDestroyOptions {
    pub batch_size: Option<usize>,
    pub return_records: bool,
    pub stop_on_error: bool,
    pub notify: bool,
}

impl Default for BulkDestroyOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
        }
    }
}

impl BulkDestroyOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = Some(size);
        self
    }

    pub fn return_records(mut self, return_records: bool) -> Self {
        self.return_records = return_records;
        self
    }

    pub fn stop_on_error(mut self, stop: bool) -> Self {
        self.stop_on_error = stop;
        self
    }

    pub fn notify(mut self, notify: bool) -> Self {
        self.notify = notify;
        self
    }
}

/// Results returned from a bulk action.
#[derive(Debug)]
pub struct BulkResult<R> {
    pub records: Vec<R>,
    pub errors: Vec<String>,
    pub error_count: usize,
    pub count: usize,
}

impl<R: Clone> Clone for BulkResult<R> {
    fn clone(&self) -> Self {
        Self {
            records: self.records.clone(),
            errors: self.errors.clone(),
            error_count: self.error_count,
            count: self.count,
        }
    }
}

impl<R> Default for BulkResult<R> {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            errors: Vec::new(),
            error_count: 0,
            count: 0,
        }
    }
}

impl<R> BulkResult<R> {
    pub fn is_success(&self) -> bool {
        self.error_count == 0
    }

    pub fn into_records(self) -> Vec<R> {
        self.records
    }
}

/// Create multiple records in a single batch or in chunked batches.
pub async fn bulk_create<R: Resource, D: DataLayer, I, F>(
    ctx: &Context<D>,
    action: &str,
    inputs: I,
    opts: BulkCreateOptions,
) -> Result<BulkResult<R>>
where
    I: IntoIterator<Item = F>,
    F: IntoFieldMap,
{
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Create)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    let pk = pk_name(&R::DEF)?;

    let mut prepared = Vec::new();
    let mut errors = Vec::new();
    let mut error_count = 0;

    for input in inputs {
        let raw_fields = input.into_field_map();
        let prep_res = (|| -> Result<(Uuid, FieldMap, FieldMap)> {
            let (mut fields, arguments) = split_input(action_def, raw_fields)?;
            generate_pk(&R::DEF, &mut fields);

            if let Some(v_attr) = R::DEF.optimistic_lock_attribute()
                && (!fields.contains_key(v_attr) || fields.get(v_attr) == Some(&Value::Null))
            {
                fields.insert(v_attr.to_string(), Value::Int(1));
            }

            for attr in R::DEF.attributes {
                if let Some(def_fn) = attr.default_fn
                    && (!fields.contains_key(attr.name) || fields.get(attr.name) == Some(&Value::Null))
                {
                    fields.insert(attr.name.to_string(), def_fn());
                }
            }

            if let Some((created_at, updated_at)) = R::DEF.timestamps {
                let now = crate::resource::utc_now_iso8601();
                if !fields.contains_key(created_at) || fields.get(created_at) == Some(&Value::Null) {
                    fields.insert(created_at.to_string(), Value::String(now.clone()));
                }
                if !fields.contains_key(updated_at) || fields.get(updated_at) == Some(&Value::Null) {
                    fields.insert(updated_at.to_string(), Value::String(now));
                }
            }

            apply_changes_with_context(
                &mut fields,
                action_def,
                ctx.actor.as_ref(),
                ctx.tenant(),
                ctx.metadata(),
                &arguments,
                &mut Vec::new(),
                &mut Vec::new(),
                &mut Vec::new(),
            )?;
            validate(&R::DEF, &fields)?;
            run_validations_with_context(
                &R::DEF,
                action_def,
                None,
                &fields,
                ctx.actor.as_ref(),
                ctx.tenant(),
                ctx.metadata(),
                &arguments,
            )?;
            authorize_field_writes(&R::DEF, ctx.actor.as_ref(), None, &fields)?;
            authorize_write(&R::DEF, action_def, ctx.actor.as_ref(), Some(&fields))?;

            let id = required_uuid(&fields, pk)?;
            Ok((id, fields, arguments))
        })();

        match prep_res {
            Ok(tuple) => prepared.push(tuple),
            Err(err) => {
                if opts.stop_on_error {
                    return Err(err);
                } else {
                    errors.push(err.to_string());
                    error_count += 1;
                }
            }
        }
    }

    let mut records = Vec::new();
    let mut count = 0;

    let chunk_size = opts.batch_size.unwrap_or(if prepared.is_empty() { 1 } else { prepared.len() });
    for chunk in prepared.chunks(chunk_size) {
        if let Some((ident_name, ref u_fields)) = opts.upsert {
            let identity = R::DEF.identity(ident_name).ok_or_else(|| {
                Error::Invalid(format!(
                    "unknown identity `{ident_name}` for upsert on {}",
                    R::DEF.name
                ))
            })?;

            for (id, fields, arguments) in chunk {
                match ctx.data.upsert(&R::DEF, *id, fields.clone(), identity, u_fields).await {
                    Ok(mut stored) => {
                        if opts.notify {
                            let mut notif_metadata = ctx.metadata.clone();
                            notif_metadata.extend(arguments.clone());
                            let notification = crate::notifier::Notification::new(
                                R::DEF.name,
                                action_def.name,
                                action_def.kind,
                                *id,
                                stored.clone(),
                                None,
                                ctx.actor.clone(),
                                notif_metadata,
                            ).with_tenant(ctx.tenant.clone());
                            crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;
                        }
                        if opts.return_records {
                            redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
                            records.push(R::from_fields(&stored)?);
                        }
                        count += 1;
                    }
                    Err(err) => {
                        if opts.stop_on_error {
                            return Err(err);
                        } else {
                            errors.push(err.to_string());
                            error_count += 1;
                        }
                    }
                }
            }
        } else {
            let chunk_tuples: Vec<(Uuid, FieldMap)> = chunk.iter().map(|(id, f, _)| (*id, f.clone())).collect();
            match ctx.data.bulk_create(&R::DEF, chunk_tuples).await {
                Ok(stored_rows) => {
                    for (mut stored, (id, _, arguments)) in stored_rows.into_iter().zip(chunk) {
                        if opts.notify {
                            let mut notif_metadata = ctx.metadata.clone();
                            notif_metadata.extend(arguments.clone());
                            let notification = crate::notifier::Notification::new(
                                R::DEF.name,
                                action_def.name,
                                action_def.kind,
                                *id,
                                stored.clone(),
                                None,
                                ctx.actor.clone(),
                                notif_metadata,
                            ).with_tenant(ctx.tenant.clone());
                            crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;
                        }
                        if opts.return_records {
                            redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
                            records.push(R::from_fields(&stored)?);
                        }
                        count += 1;
                    }
                }
                Err(err) => {
                    if opts.stop_on_error {
                        return Err(err);
                    } else {
                        errors.push(err.to_string());
                        error_count += chunk.len();
                    }
                }
            }
        }
    }

    Ok(BulkResult {
        records,
        errors,
        error_count,
        count,
    })
}

/// Destroy multiple records by ID in a single batch or in chunked batches.
pub async fn bulk_destroy<R: Resource, D: DataLayer>(
    ctx: &Context<D>,
    action: &str,
    ids: &[Uuid],
    opts: BulkDestroyOptions,
) -> Result<BulkResult<R>> {
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Destroy)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    if ids.is_empty() {
        return Ok(BulkResult::default());
    }

    let pk = pk_name(&R::DEF)?;

    // Fetch existing records for authorization, cascading deletes, and notifications
    let filter = Filter::In(pk.to_string(), ids.iter().map(|id| Value::Uuid(*id)).collect());
    let rows = ctx
        .data
        .run_query(
            &R::DEF,
            &CompiledQuery {
                filter: Some(filter),
                tenant: ctx.tenant.clone(),
                ..CompiledQuery::default()
            },
        )
        .await?;

    let mut valid_to_destroy = Vec::new();
    let mut errors = Vec::new();
    let mut error_count = 0;

    for row in rows {
        let id = required_uuid(&row, pk)?;

        let check_res = (|| -> Result<()> {
            authorize_write(&R::DEF, action_def, ctx.actor.as_ref(), Some(&row))?;
            Ok(())
        })();

        match check_res {
            Ok(()) => {
                // Check cascading deletes
                match crate::engine::handle_cascading_deletes(ctx, &R::DEF, id, &row).await {
                    Ok(()) => valid_to_destroy.push((id, row)),
                    Err(err) => {
                        if opts.stop_on_error {
                            return Err(err);
                        } else {
                            errors.push(err.to_string());
                            error_count += 1;
                        }
                    }
                }
            }
            Err(err) => {
                if opts.stop_on_error {
                    return Err(err);
                } else {
                    errors.push(err.to_string());
                    error_count += 1;
                }
            }
        }
    }

    let mut records = Vec::new();
    let mut count = 0;

    let chunk_size = opts.batch_size.unwrap_or(if valid_to_destroy.is_empty() { 1 } else { valid_to_destroy.len() });
    for chunk in valid_to_destroy.chunks(chunk_size) {
        let chunk_ids: Vec<Uuid> = chunk.iter().map(|(id, _)| *id).collect();
        match ctx.data.bulk_destroy(&R::DEF, &chunk_ids).await {
            Ok(()) => {
                for (id, row) in chunk {
                    if opts.notify {
                        let notification = crate::notifier::Notification::new(
                            R::DEF.name,
                            action_def.name,
                            action_def.kind,
                            *id,
                            row.clone(),
                            Some(row.clone()),
                            ctx.actor.clone(),
                            ctx.metadata.clone(),
                        ).with_tenant(ctx.tenant.clone());
                        crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;
                    }
                    if opts.return_records {
                        records.push(R::from_fields(row)?);
                    }
                    count += 1;
                }
            }
            Err(err) => {
                if opts.stop_on_error {
                    return Err(err);
                } else {
                    errors.push(err.to_string());
                    error_count += chunk.len();
                }
            }
        }
    }

    Ok(BulkResult {
        records,
        errors,
        error_count,
        count,
    })
}
