use uuid::Uuid;

use crate::action::{ActionKind, PersistKind};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::action::DynamicAfterTransactionHook;
use crate::changeset::DynamicChangeset;
use crate::changeset::dynamic_upsert_identity as upsert_identity;
use crate::pipeline::{action_named, expect_kind, expect_persist, pk_name, visible_scope};
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

/// Options for a bulk update.
#[derive(Clone, Debug)]
pub struct BulkUpdateOptions {
    pub batch_size: Option<usize>,
    pub return_records: bool,
    pub stop_on_error: bool,
    pub notify: bool,
}

impl Default for BulkUpdateOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
        }
    }
}

impl BulkUpdateOptions {
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

/// One row of a bulk action, prepared as a single write prepares, with the hooks to run
/// once its outcome is known.
struct PreparedRow {
    id: Uuid,
    changeset: DynamicChangeset,
    after_transactions: Vec<DynamicAfterTransactionHook>,
}

/// Runs `hooks` with a row's outcome.
fn conclude(hooks: Vec<DynamicAfterTransactionHook>, outcome: std::result::Result<&FieldMap, &Error>) {
    for hook in hooks {
        hook(outcome);
    }
}

impl<R> BulkResult<R> {
    /// Counts a failed row, or returns its error when the action stops on the first one.
    fn fail(&mut self, err: Error, stop_on_error: bool) -> Result<()> {
        if stop_on_error {
            return Err(err);
        }
        self.errors.push(err.to_string());
        self.error_count += 1;
        Ok(())
    }
}

/// Create multiple records in a single batch or in chunked batches.
///
/// Each row runs through the same changeset as a single create: accept, changes,
/// tenant, validations, policies and before-action hooks, then after-action hooks and a
/// notification once its batch is written.
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
    if let Some((identity, _)) = &opts.upsert {
        upsert_identity(&R::DEF, identity)?;
    }

    let mut result = BulkResult::default();
    let mut prepared = Vec::new();
    for input in inputs {
        let mut changeset =
            match DynamicChangeset::for_create(ctx, &R::DEF, action_def, input.into_field_map()) {
                Ok(changeset) => changeset,
                Err(err) => {
                    result.fail(err, opts.stop_on_error)?;
                    continue;
                }
            };
        if let Some((identity, update_fields)) = &opts.upsert {
            let update_fields: Vec<&str> = update_fields.iter().map(String::as_str).collect();
            changeset = changeset.with_upsert(identity, &update_fields);
        }
        let after_transactions = changeset.take_after_transactions();
        match changeset.prepare(ctx).await {
            Ok(id) => prepared.push(PreparedRow {
                id,
                changeset,
                after_transactions,
            }),
            Err(err) => {
                conclude(after_transactions, Err(&err));
                result.fail(err, opts.stop_on_error)?;
            }
        }
    }

    let cascade = crate::engine::Cascade::new(opts.notify);
    let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
    let mut rows = prepared.into_iter().peekable();
    while rows.peek().is_some() {
        let mut chunk: Vec<PreparedRow> = rows.by_ref().take(chunk_size).collect();
        // Upserts go one row at a time, so one conflict fails only its own row.
        let stored: Vec<Result<FieldMap>> = if opts.upsert.is_some() {
            let mut stored = Vec::with_capacity(chunk.len());
            for row in &mut chunk {
                stored.push(row.changeset.persist(ctx, row.id, &cascade).await);
            }
            stored
        } else {
            let tuples = chunk
                .iter_mut()
                .map(|row| (row.id, row.changeset.take_fields()))
                .collect();
            match ctx.data.bulk_create(&R::DEF, ctx.tenant.as_deref(), tuples).await {
                Ok(stored) => stored.into_iter().map(Ok).collect(),
                Err(err) => {
                    let failed = chunk.len();
                    for row in chunk {
                        conclude(row.after_transactions, Err(&err));
                    }
                    if opts.stop_on_error {
                        return Err(err);
                    }
                    result.errors.push(err.to_string());
                    result.error_count += failed;
                    continue;
                }
            }
        };

        for (mut row, stored) in chunk.into_iter().zip(stored) {
            let outcome = match stored {
                Ok(stored) => row.changeset.finish(ctx, row.id, stored, opts.notify).await,
                Err(err) => Err(err),
            };
            conclude(row.after_transactions, outcome.as_ref());
            match outcome {
                Ok(stored) => {
                    if opts.return_records {
                        result.records.push(R::from_fields(&stored)?);
                    }
                    result.count += 1;
                }
                Err(err) => result.fail(err, opts.stop_on_error)?,
            }
        }
    }

    Ok(result)
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
                Ok(stored) => {
                    if opts.return_records {
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

    // Each row runs through the same changeset as a single destroy, then the rows are
    // deleted together.
    let mut result = BulkResult::default();
    let mut prepared = Vec::new();
    for row in rows {
        let mut changeset = match DynamicChangeset::for_destroy(ctx, &R::DEF, action_def, row) {
            Ok(changeset) => changeset,
            Err(err) => {
                result.fail(err, opts.stop_on_error)?;
                continue;
            }
        };
        let after_transactions = changeset.take_after_transactions();
        let ready = async {
            let id = changeset.prepare(ctx).await?;
            let existing = changeset.existing().cloned().unwrap_or_default();
            crate::engine::handle_cascading_deletes(ctx, &R::DEF, id, &existing).await?;
            Ok::<_, Error>((id, existing))
        }
        .await;
        match ready {
            Ok((id, existing)) => prepared.push((
                PreparedRow {
                    id,
                    changeset,
                    after_transactions,
                },
                existing,
            )),
            Err(err) => {
                conclude(after_transactions, Err(&err));
                result.fail(err, opts.stop_on_error)?;
            }
        }
    }

    let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
    let mut rows = prepared.into_iter().peekable();
    while rows.peek().is_some() {
        let chunk: Vec<(PreparedRow, FieldMap)> = rows.by_ref().take(chunk_size).collect();
        let ids: Vec<Uuid> = chunk.iter().map(|(row, _)| row.id).collect();
        if let Err(err) = ctx.data.bulk_destroy(&R::DEF, ctx.tenant.as_deref(), &ids).await {
            let failed = chunk.len();
            for (row, _) in chunk {
                conclude(row.after_transactions, Err(&err));
            }
            if opts.stop_on_error {
                return Err(err);
            }
            result.errors.push(err.to_string());
            result.error_count += failed;
            continue;
        }
        for (mut row, existing) in chunk {
            let outcome = row.changeset.finish(ctx, row.id, existing, opts.notify).await;
            conclude(row.after_transactions, outcome.as_ref());
            match outcome {
                Ok(stored) => {
                    if opts.return_records {
                        result.records.push(R::from_fields(&stored)?);
                    }
                    result.count += 1;
                }
                Err(err) => result.fail(err, opts.stop_on_error)?,
            }
        }
    }

    Ok(result)
}

/// Update several records, each with its own input, through one update action.
///
/// Each record runs through the same changeset as a single update made from it: accept,
/// changes, validations, policies and before-action hooks. Each batch is then written
/// together, the attributes each row changes, in one statement where the data layer can
/// (`DataLayer::bulk_update`). Each row then runs its after-action hooks and is notified
/// as an update. Ash's `bulk_update` applies one input to every record it's given; this
/// takes an input per record, as a stream of telemetry does, where every record reports
/// its own values.
pub async fn bulk_update<R: Resource, D: DataLayer, I, F>(
    ctx: &Context<D>,
    action: &str,
    updates: I,
    opts: BulkUpdateOptions,
) -> Result<BulkResult<R>>
where
    I: IntoIterator<Item = (R, F)>,
    F: IntoFieldMap,
{
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Update)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    let mut result = BulkResult::default();
    let mut prepared = Vec::new();
    for (record, input) in updates {
        let mut changeset = match DynamicChangeset::for_update(
            ctx,
            &R::DEF,
            action_def,
            record.to_fields(),
            input.into_field_map(),
        ) {
            Ok(changeset) => changeset,
            Err(err) => {
                result.fail(err, opts.stop_on_error)?;
                continue;
            }
        };
        let after_transactions = changeset.take_after_transactions();
        match changeset.prepare(ctx).await {
            Ok(id) => prepared.push(PreparedRow {
                id,
                changeset,
                after_transactions,
            }),
            Err(err) => {
                conclude(after_transactions, Err(&err));
                result.fail(err, opts.stop_on_error)?;
            }
        }
    }

    let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
    let mut rows = prepared.into_iter().peekable();
    while rows.peek().is_some() {
        let mut chunk: Vec<PreparedRow> = rows.by_ref().take(chunk_size).collect();
        let writes = chunk
            .iter_mut()
            .map(|row| {
                let fields = row.changeset.take_fields();
                (row.id, row.changeset.changes(fields))
            })
            .collect();
        let stored = match ctx.data.bulk_update(&R::DEF, ctx.tenant.as_deref(), writes).await {
            Ok(stored) => stored,
            Err(err) => {
                let failed = chunk.len();
                for row in chunk {
                    conclude(row.after_transactions, Err(&err));
                }
                if opts.stop_on_error {
                    return Err(err);
                }
                result.errors.push(err.to_string());
                result.error_count += failed;
                continue;
            }
        };
        for (mut row, stored) in chunk.into_iter().zip(stored) {
            let outcome = match stored {
                Ok(stored) => row.changeset.finish(ctx, row.id, stored, opts.notify).await,
                Err(err) => Err(err),
            };
            conclude(row.after_transactions, outcome.as_ref());
            match outcome {
                Ok(stored) => {
                    if opts.return_records {
                        result.records.push(R::from_fields(&stored)?);
                    }
                    result.count += 1;
                }
                Err(err) => result.fail(err, opts.stop_on_error)?,
            }
        }
    }

    Ok(result)
}
