use uuid::Uuid;

use crate::action::{ActionKind, PersistKind};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{
    action_named, apply_changes_with_context, apply_tenant_to_fields, expect_kind, expect_persist,
    pk_name, prepare_create_fields, run_validations_with_context, split_input, validate,
    visible_scope,
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
/// One validated row of a bulk create, with the hooks its action's changes registered.
struct PreparedRow {
    id: Uuid,
    fields: FieldMap,
    arguments: FieldMap,
    after_actions: Vec<crate::action::DynamicAfterActionHook>,
    after_transactions: Vec<crate::action::DynamicAfterTransactionHook>,
}

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

    let mut prepared: Vec<PreparedRow> = Vec::new();
    let mut errors = Vec::new();
    let mut error_count = 0;

    for input in inputs {
        let raw_fields = input.into_field_map();
        let prep_res = (|| -> Result<PreparedRow> {
            // The same steps, in the same order, as a single create (`create_dynamic`).
            let (mut fields, arguments) = split_input(action_def, raw_fields)?;
            prepare_create_fields(&R::DEF, &mut fields);
            let mut before_actions = Vec::new();
            let mut after_actions = Vec::new();
            let mut after_transactions = Vec::new();
            apply_changes_with_context(
                &mut fields,
                action_def,
                ctx.actor.as_ref(),
                ctx.tenant(),
                ctx.metadata(),
                &arguments,
                &mut before_actions,
                &mut after_actions,
                &mut after_transactions,
            )?;
            apply_tenant_to_fields(&R::DEF, &mut fields, ctx.tenant(), true)?;
            let check = |fields: &mut FieldMap| -> Result<()> {
                run_validations_with_context(
                    &R::DEF,
                    action_def,
                    None,
                    fields,
                    ctx.actor.as_ref(),
                    ctx.tenant(),
                    ctx.metadata(),
                    &arguments,
                )?;
                validate(&R::DEF, fields)
            };
            check(&mut fields)?;
            authorize_field_writes(&R::DEF, ctx.actor.as_ref(), None, &fields)?;
            authorize_write(&R::DEF, action_def, ctx.actor.as_ref(), Some(&fields))?;
            for hook in before_actions {
                hook(&mut fields)?;
            }
            check(&mut fields)?;

            let id = required_uuid(&fields, pk)?;
            Ok(PreparedRow {
                id,
                fields,
                arguments,
                after_actions,
                after_transactions,
            })
        })();

        match prep_res {
            Ok(row) => prepared.push(row),
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

    let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
    let upsert_identity = match opts.upsert {
        Some((ident_name, ref update_fields)) => Some((
            R::DEF.identity(ident_name).ok_or_else(|| {
                Error::Invalid(format!(
                    "unknown identity `{ident_name}` for upsert on {}",
                    R::DEF.name
                ))
            })?,
            update_fields.clone(),
        )),
        None => None,
    };
    let mut rows = prepared.into_iter().peekable();
    while rows.peek().is_some() {
        // Rows are written while hooks wait, so keep the hooks apart from what is borrowed
        // across the writes.
        let (chunk, hooks): (Vec<_>, Vec<_>) = rows
            .by_ref()
            .take(chunk_size)
            .map(|row| {
                (
                    (row.id, row.fields, row.arguments),
                    (row.after_actions, row.after_transactions),
                )
            })
            .unzip();
        let stored_rows: Result<Vec<Result<FieldMap>>> = match &upsert_identity {
            // Upserts go one row at a time, so one conflict fails only its own row.
            Some((identity, update_fields)) => {
                let mut stored = Vec::with_capacity(chunk.len());
                for (id, fields, _) in &chunk {
                    stored.push(
                        ctx.data
                            .upsert(&R::DEF, *id, fields.clone(), identity, update_fields)
                            .await,
                    );
                }
                Ok(stored)
            }
            None => {
                let tuples: Vec<(Uuid, FieldMap)> = chunk
                    .iter()
                    .map(|(id, fields, _)| (*id, fields.clone()))
                    .collect();
                ctx.data
                    .bulk_create(&R::DEF, tuples)
                    .await
                    .map(|stored| stored.into_iter().map(Ok).collect())
            }
        };
        let stored_rows = match stored_rows {
            Ok(stored_rows) => stored_rows,
            Err(err) => {
                for (_, after_transactions) in hooks {
                    for hook in after_transactions {
                        hook(Err(&err));
                    }
                }
                if opts.stop_on_error {
                    return Err(err);
                }
                errors.push(err.to_string());
                error_count += chunk.len();
                continue;
            }
        };

        for ((stored, (id, _, arguments)), (after_actions, after_transactions)) in
            stored_rows.into_iter().zip(chunk).zip(hooks)
        {
            let outcome = stored.and_then(|mut stored| {
                for hook in after_actions {
                    hook(&mut stored)?;
                }
                Ok(stored)
            });
            match outcome {
                Ok(mut stored) => {
                    for hook in after_transactions {
                        hook(Ok(&stored));
                    }
                    if opts.notify {
                        let mut notif_metadata = ctx.metadata.clone();
                        notif_metadata.extend(arguments);
                        let notification = crate::notifier::Notification::new(
                            R::DEF.name,
                            action_def.name,
                            action_def.kind,
                            id,
                            stored.clone(),
                            None,
                            ctx.actor.clone(),
                            notif_metadata,
                        )
                        .with_tenant(ctx.tenant.clone());
                        crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;
                    }
                    if opts.return_records {
                        redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
                        records.push(R::from_fields(&stored)?);
                    }
                    count += 1;
                }
                Err(err) => {
                    for hook in after_transactions {
                        hook(Err(&err));
                    }
                    if opts.stop_on_error {
                        return Err(err);
                    }
                    errors.push(err.to_string());
                    error_count += 1;
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

    // Fetch existing records for authorization, cascading deletes, and notifications.
    // Tenant scope must match Query::load so knowing a UUID is not enough to delete across tenants.
    let id_filter = Filter::In(
        pk.to_string(),
        ids.iter().map(|id| Value::Uuid(*id)).collect(),
    );
    let (filter, tenant) = visible_scope(&R::DEF, Some(id_filter), ctx.tenant.clone())?;
    let rows = ctx
        .data
        .run_query(
            &R::DEF,
            &CompiledQuery {
                filter,
                tenant,
                ..CompiledQuery::default()
            },
        )
        .await?;

    // Soft and cascading destroys run each record's changes and cascades, so they go
    // one record at a time instead of through a single bulk delete.
    if action_def.soft || !action_def.cascade_destroy.is_empty() {
        let mut result = BulkResult::default();
        let cascade = crate::engine::Cascade::new(opts.notify);
        for row in rows {
            let id = required_uuid(&row, pk)?;
            let destroyed =
                crate::engine::destroy_dynamic_with(ctx, &R::DEF, action_def, id, &row, &cascade)
                    .await;
            match destroyed {
                Ok(mut stored) => {
                    if opts.return_records {
                        crate::policy::redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
                        result.records.push(R::from_fields(&stored)?);
                    }
                    result.count += 1;
                }
                Err(err) if opts.stop_on_error => return Err(err),
                Err(err) => {
                    result.errors.push(err.to_string());
                    result.error_count += 1;
                }
            }
        }
        return Ok(result);
    }

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

    let chunk_size = opts.batch_size.unwrap_or(if valid_to_destroy.is_empty() {
        1
    } else {
        valid_to_destroy.len()
    });
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
                        )
                        .with_tenant(ctx.tenant.clone());
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
