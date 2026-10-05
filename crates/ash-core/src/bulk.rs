
use crate::action::{ActionKind, PersistKind};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{CompiledQuery, DataLayer, TransactionSupport};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::action::DynamicAfterTransactionHook;
use crate::changeset::DynamicChangeset;
use crate::changeset::dynamic_upsert_identity as upsert_identity;
use crate::pipeline::{action_named, expect_kind, expect_persist, pk_name, visible_scope};
use crate::resource::Resource;
use crate::value::{FieldMap, Value};

/// How a bulk action uses transactions: Ash's `transaction` option.
///
/// A data layer that can't transact runs every batch without one, as Ash asks
/// `data_layer_can?(resource, :transact)` first: ash-memory can't, as Ash's ETS layer
/// can't.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BulkTransaction {
    /// The whole action in one transaction (`transaction: :all`). After-transaction
    /// hooks run inside it, as Ash runs them with `:all`.
    All,
    /// Each batch in its own transaction (`transaction: :batch`, Ash's default): a batch
    /// is written, its rows finished and its notifications sent once it commits.
    #[default]
    Batch,
    /// No transaction (`transaction: false`): each row stands alone.
    Off,
}

/// Options for configuring a bulk create operation.
#[derive(Clone, Debug)]
pub struct BulkCreateOptions {
    pub batch_size: Option<usize>,
    pub return_records: bool,
    pub stop_on_error: bool,
    pub notify: bool,
    pub upsert: Option<(&'static str, Vec<String>)>,
    pub transaction: BulkTransaction,
}

impl Default for BulkCreateOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
            upsert: None,
            transaction: BulkTransaction::default(),
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

    pub fn transaction(mut self, transaction: BulkTransaction) -> Self {
        self.transaction = transaction;
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
    pub transaction: BulkTransaction,
}

impl Default for BulkDestroyOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
            transaction: BulkTransaction::default(),
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

    pub fn transaction(mut self, transaction: BulkTransaction) -> Self {
        self.transaction = transaction;
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
    pub transaction: BulkTransaction,
}

impl Default for BulkUpdateOptions {
    fn default() -> Self {
        Self {
            batch_size: None,
            return_records: true,
            stop_on_error: true,
            notify: true,
            transaction: BulkTransaction::default(),
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

    pub fn transaction(mut self, transaction: BulkTransaction) -> Self {
        self.transaction = transaction;
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
    id: Value,
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

/// Where a bulk action stands with transactions, once the data layer has said whether it
/// can transact.
#[derive(Clone, Copy)]
struct Scope {
    /// Each batch opens a transaction of its own.
    per_batch: bool,
    /// The whole action runs in one.
    whole: bool,
    stop_on_error: bool,
}

impl Scope {
    fn new<D: TransactionSupport>(ctx: &Context<D>, transaction: BulkTransaction, stop_on_error: bool) -> Self {
        let can = ctx.data.can_transact();
        Self {
            per_batch: can && transaction == BulkTransaction::Batch,
            whole: can && transaction == BulkTransaction::All,
            stop_on_error,
        }
    }

    /// Whether the rows are written in a transaction, so a failed row rolls back the rest
    /// with it, as Ash rolls back on error (`rollback_on_error?`, true by default). A
    /// statement that fails also leaves a Postgres transaction unable to commit, so a row
    /// can't fail alone inside one.
    fn transactional(&self) -> bool {
        self.per_batch || self.whole
    }

    /// Fails a row that couldn't be prepared. Rows are prepared before their batch opens,
    /// so a failure there rolls back nothing unless the whole action is one transaction.
    fn reject<R>(&self, result: &mut BulkResult<R>, err: Error) -> Result<()> {
        if self.whole {
            return Err(err);
        }
        result.fail(err, self.stop_on_error)
    }
}

/// Runs `action` over the whole bulk action, in one transaction when the scope says so.
/// Rolled back, every row fails with the error that rolled it back.
async fn in_scope<R, D, F, Fut>(
    ctx: &Context<D>,
    scope: Scope,
    rows: usize,
    action: F,
) -> Result<BulkResult<R>>
where
    R: Resource,
    D: TransactionSupport + 'static,
    F: FnOnce(Context<D>) -> Fut + Send,
    Fut: Future<Output = Result<BulkResult<R>>> + Send,
{
    if !scope.whole {
        return action(ctx.clone()).await;
    }
    match ctx.transaction(action).await {
        Ok(result) => Ok(result),
        Err(err) if scope.stop_on_error => Err(err),
        Err(err) => Ok(BulkResult {
            errors: vec![err.to_string()],
            error_count: rows,
            ..BulkResult::default()
        }),
    }
}

/// Writes one batch: in a transaction of its own when the scope says so, which commits
/// before the batch's notifications are sent. In a transaction, the first row that fails
/// ends the batch and rolls it back.
async fn in_batch<D, F, Fut>(ctx: &Context<D>, scope: Scope, write: F) -> Result<Vec<Result<FieldMap>>>
where
    D: TransactionSupport + 'static,
    F: FnOnce(Context<D>) -> Fut + Send,
    Fut: Future<Output = Result<Vec<Result<FieldMap>>>> + Send,
{
    if scope.per_batch {
        ctx.transaction(write).await
    } else {
        write(ctx.clone()).await
    }
}

/// Finishes each row of a written batch, in order: its after-action hooks and its
/// notification. In a transaction, the first row that fails, in the data layer or here,
/// fails the batch.
async fn finish_rows<D: DataLayer>(
    ctx: &Context<D>,
    rows: Vec<(Value, DynamicChangeset)>,
    stored: Vec<Result<FieldMap>>,
    notify: bool,
    transactional: bool,
) -> Result<Vec<Result<FieldMap>>> {
    let mut outcomes = Vec::with_capacity(rows.len());
    for ((id, mut changeset), stored) in rows.into_iter().zip(stored) {
        let outcome = match stored {
            Ok(stored) => changeset.finish(ctx, id, stored, notify).await,
            Err(err) => Err(err),
        };
        match outcome {
            // A record changed since it was read is left out, as Ash's bulk actions leave
            // a stale record out; it doesn't undo the batch.
            Err(Error::StaleRecord { .. }) => outcomes.push(outcome),
            Err(err) if transactional => return Err(err),
            outcome => outcomes.push(outcome),
        }
    }
    Ok(outcomes)
}

/// Concludes a batch's rows with their outcomes, then counts them. A batch that failed as
/// a whole fails every row in it; inside a whole-action transaction, it rolls that back.
fn settle<R: Resource>(
    result: &mut BulkResult<R>,
    scope: Scope,
    return_records: bool,
    hooks: Vec<Vec<DynamicAfterTransactionHook>>,
    outcomes: Result<Vec<Result<FieldMap>>>,
) -> Result<()> {
    let outcomes = match outcomes {
        Ok(outcomes) => outcomes,
        Err(err) => {
            let failed = hooks.len();
            for row in hooks {
                conclude(row, Err(&err));
            }
            if scope.whole || scope.stop_on_error {
                return Err(err);
            }
            result.errors.push(err.to_string());
            result.error_count += failed;
            return Ok(());
        }
    };
    for (row, outcome) in hooks.into_iter().zip(&outcomes) {
        conclude(row, outcome.as_ref());
    }
    for outcome in outcomes {
        match outcome {
            Ok(stored) => {
                if return_records {
                    result.records.push(R::from_fields(&stored)?);
                }
                result.count += 1;
            }
            Err(Error::StaleRecord { .. }) => {}
            Err(err) => result.fail(err, scope.stop_on_error)?,
        }
    }
    Ok(())
}

/// Splits prepared rows into what their batch writes and the hooks that wait on it.
fn split(chunk: Vec<PreparedRow>) -> (Vec<(Value, DynamicChangeset)>, Vec<Vec<DynamicAfterTransactionHook>>) {
    chunk
        .into_iter()
        .map(|row| ((row.id, row.changeset), row.after_transactions))
        .unzip()
}

/// Create multiple records in a single batch or in chunked batches.
///
/// Each row runs through the same changeset as a single create: accept, changes,
/// tenant, validations, policies and before-action hooks, then after-action hooks and a
/// notification once its batch is written. Each batch is written in a transaction, as
/// Ash's are, unless [`BulkCreateOptions::transaction`] says otherwise.
pub async fn bulk_create<R: Resource, D: TransactionSupport + 'static, I, F>(
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
    let inputs: Vec<FieldMap> = inputs.into_iter().map(IntoFieldMap::into_field_map).collect();
    let scope = Scope::new(ctx, opts.transaction, opts.stop_on_error);
    in_scope(ctx, scope, inputs.len(), move |ctx| async move {
        let mut result = BulkResult::default();
        let mut prepared = Vec::new();
        for input in inputs {
            let mut changeset = match DynamicChangeset::for_create(&ctx, &R::DEF, action_def, input) {
                Ok(changeset) => changeset,
                Err(err) => {
                    scope.reject(&mut result, err)?;
                    continue;
                }
            };
            if let Some((identity, update_fields)) = &opts.upsert {
                let update_fields: Vec<&str> = update_fields.iter().map(String::as_str).collect();
                changeset = changeset.with_upsert(identity, &update_fields);
            }
            let after_transactions = changeset.take_after_transactions();
            match changeset.prepare(&ctx).await {
                Ok(id) => prepared.push(PreparedRow {
                    id,
                    changeset,
                    after_transactions,
                }),
                Err(err) => {
                    conclude(after_transactions, Err(&err));
                    scope.reject(&mut result, err)?;
                }
            }
        }

        let (notify, upsert) = (opts.notify, opts.upsert.is_some());
        let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
        let mut rows = prepared.into_iter().peekable();
        while rows.peek().is_some() {
            let (mut batch, hooks) = split(rows.by_ref().take(chunk_size).collect());
            let outcomes = in_batch(&ctx, scope, move |ctx| async move {
                // Upserts go one row at a time, so one conflict fails only its own row
                // when there's no transaction for it to roll back.
                let stored: Vec<Result<FieldMap>> = if upsert {
                    let cascade = crate::engine::Cascade::new(notify);
                    let mut stored = Vec::with_capacity(batch.len());
                    for (id, changeset) in &mut batch {
                        stored.push(changeset.persist(&ctx, id.clone(), &cascade).await);
                    }
                    stored
                } else {
                    let tuples = batch
                        .iter_mut()
                        .map(|(id, changeset)| (id.clone(), changeset.take_fields()))
                        .collect();
                    let stored = ctx.data.bulk_create(&R::DEF, ctx.tenant.as_deref(), tuples).await?;
                    stored.into_iter().map(Ok).collect()
                };
                finish_rows(&ctx, batch, stored, notify, scope.transactional()).await
            })
            .await;
            settle(&mut result, scope, opts.return_records, hooks, outcomes)?;
        }
        Ok(result)
    })
    .await
}

/// Destroy multiple records by ID in a single batch or in chunked batches, each batch in a
/// transaction, as Ash's are, unless [`BulkDestroyOptions::transaction`] says otherwise.
pub async fn bulk_destroy<R: Resource, D: TransactionSupport + 'static, I: Clone + Into<Value>>(
    ctx: &Context<D>,
    action: &str,
    ids: &[I],
    opts: BulkDestroyOptions,
) -> Result<BulkResult<R>> {
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Destroy)?;
    expect_persist(action_def, PersistKind::DataLayer)?;

    if ids.is_empty() {
        return Ok(BulkResult::default());
    }

    let pk = pk_name(&R::DEF)?;
    let ids: Vec<Value> = ids.iter().cloned().map(Into::into).collect();
    let scope = Scope::new(ctx, opts.transaction, opts.stop_on_error);
    in_scope(ctx, scope, ids.len(), move |ctx| async move {
        // Fetch existing records for authorization, cascading deletes, and notifications.
        // Tenant scope must match Query::load so knowing a UUID is not enough to delete across tenants.
        let id_filter = Filter::In(pk.to_string(), ids);
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
        let chunk_size = opts.batch_size.unwrap_or(rows.len()).max(1);
        let notify = opts.notify;
        let mut result = BulkResult::default();

        // Soft and cascading destroys run each record's changes and cascades, so they go
        // one record at a time instead of through a single bulk delete.
        if action_def.soft || !action_def.cascade_destroy.is_empty() {
            let mut rows = rows.into_iter().peekable();
            while rows.peek().is_some() {
                let batch: Vec<FieldMap> = rows.by_ref().take(chunk_size).collect();
                let hooks = (0..batch.len()).map(|_| Vec::new()).collect();
                let outcomes = in_batch(&ctx, scope, move |ctx| async move {
                    let cascade = crate::engine::Cascade::new(notify);
                    let mut outcomes = Vec::with_capacity(batch.len());
                    for row in batch {
                        let id = crate::value::required_pk(&row, pk)?;
                        // A record changed since it was read is left out before anything
                        // runs for it, as Ash's filter leaves a stale record out.
                        if let Some(lock) = crate::engine::lock_of(action_def, &row)
                            && let Err(err) = crate::engine::check_lock(&ctx, &R::DEF, id.clone(), &lock).await
                        {
                            match err {
                                Error::StaleRecord { .. } => outcomes.push(Err(err)),
                                err if scope.transactional() => return Err(err),
                                err => outcomes.push(Err(err)),
                            }
                            continue;
                        }
                        let destroyed = crate::engine::destroy_dynamic_with(
                            &ctx, &R::DEF, action_def, id, &row, &cascade,
                        )
                        .await;
                        match destroyed {
                            Err(err) if scope.transactional() => return Err(err),
                            destroyed => outcomes.push(destroyed),
                        }
                    }
                    Ok(outcomes)
                })
                .await;
                settle(&mut result, scope, opts.return_records, hooks, outcomes)?;
            }
            return Ok(result);
        }

        // Each row runs through the same changeset as a single destroy, then the rows are
        // deleted together.
        let mut prepared = Vec::new();
        for row in rows {
            let mut changeset = match DynamicChangeset::for_destroy(&ctx, &R::DEF, action_def, row) {
                Ok(changeset) => changeset,
                Err(err) => {
                    scope.reject(&mut result, err)?;
                    continue;
                }
            };
            let after_transactions = changeset.take_after_transactions();
            match changeset.prepare(&ctx).await {
                Ok(id) => prepared.push(PreparedRow {
                    id,
                    changeset,
                    after_transactions,
                }),
                Err(err) => {
                    conclude(after_transactions, Err(&err));
                    scope.reject(&mut result, err)?;
                }
            }
        }

        let mut rows = prepared.into_iter().peekable();
        while rows.peek().is_some() {
            let (batch, hooks) = split(rows.by_ref().take(chunk_size).collect());
            let outcomes = in_batch(&ctx, scope, move |ctx| async move {
                // Each row's relationships go as it does, in the same transaction. Outside
                // one, a row whose relationships can't go fails alone and stays.
                let parents: Vec<(Value, FieldMap)> = batch
                    .iter()
                    .map(|(id, changeset)| (id.clone(), changeset.existing().cloned().unwrap_or_default()))
                    .collect();
                let mut existing = Vec::with_capacity(batch.len());
                let mut ids = Vec::with_capacity(batch.len());
                for (id, fields) in parents {
                    // A stale record is left out before its related records go.
                    if let Some(lock) = crate::engine::lock_of(action_def, &fields)
                        && let Err(err) = crate::engine::check_lock(&ctx, &R::DEF, id.clone(), &lock).await
                    {
                        match err {
                            Error::StaleRecord { .. } => existing.push(Err(err)),
                            err if scope.transactional() => return Err(err),
                            err => existing.push(Err(err)),
                        }
                        continue;
                    }
                    match crate::engine::handle_cascading_deletes(&ctx, &R::DEF, id.clone(), &fields).await {
                        Ok(()) => {
                            ids.push(id);
                            existing.push(Ok(fields));
                        }
                        Err(err) if scope.transactional() => return Err(err),
                        Err(err) => existing.push(Err(err)),
                    }
                }
                if action_def.has_optimistic_lock() {
                    // Each record as it was read, under the action's lock.
                    let cascades = crate::engine::cascades(&R::DEF, action_def);
                    for row in existing.iter_mut() {
                        let Ok(fields) = row else { continue };
                        let id = crate::pipeline::pk_name(&R::DEF).and_then(|pk| crate::value::required_pk(fields, pk))?;
                        let lock = crate::engine::lock_of(action_def, fields);
                        match crate::engine::destroy_guarded(&ctx, &R::DEF, id, lock.as_ref()).await {
                            Ok(()) => {}
                            // Changed after its lock was checked and its related records
                            // went: those must come back, so the batch fails.
                            Err(err) if cascades && scope.transactional() => return Err(err),
                            Err(err) => *row = Err(err),
                        }
                    }
                } else {
                    ctx.data.bulk_destroy(&R::DEF, ctx.tenant.as_deref(), &ids).await?;
                }
                finish_rows(&ctx, batch, existing, notify, scope.transactional()).await
            })
            .await;
            settle(&mut result, scope, opts.return_records, hooks, outcomes)?;
        }
        Ok(result)
    })
    .await
}

/// Update several records, each with its own input, through one update action.
///
/// Each record runs through the same changeset as a single update made from it: accept,
/// changes, validations, policies and before-action hooks. Each batch is then written
/// together, the attributes each row changes, in one statement where the data layer can
/// (`DataLayer::bulk_update`), in a transaction as Ash writes a batch unless
/// [`BulkUpdateOptions::transaction`] says otherwise. Each row then runs its after-action
/// hooks and is notified as an update. Ash's `bulk_update` applies one input to every
/// record it's given; this takes an input per record, as a stream of telemetry does,
/// where every record reports its own values.
pub async fn bulk_update<R: Resource, D: TransactionSupport + 'static, I, F>(
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

    let updates: Vec<(FieldMap, FieldMap)> = updates
        .into_iter()
        .map(|(record, input)| (record.to_fields(), input.into_field_map()))
        .collect();
    let scope = Scope::new(ctx, opts.transaction, opts.stop_on_error);
    in_scope(ctx, scope, updates.len(), move |ctx| async move {
        let mut result = BulkResult::default();
        let mut prepared = Vec::new();
        for (record, input) in updates {
            let mut changeset = match DynamicChangeset::for_update(&ctx, &R::DEF, action_def, record, input) {
                Ok(changeset) => changeset,
                Err(err) => {
                    scope.reject(&mut result, err)?;
                    continue;
                }
            };
            let after_transactions = changeset.take_after_transactions();
            match changeset.prepare(&ctx).await {
                Ok(id) => prepared.push(PreparedRow {
                    id,
                    changeset,
                    after_transactions,
                }),
                Err(err) => {
                    conclude(after_transactions, Err(&err));
                    scope.reject(&mut result, err)?;
                }
            }
        }

        let notify = opts.notify;
        let chunk_size = opts.batch_size.unwrap_or(prepared.len()).max(1);
        let mut rows = prepared.into_iter().peekable();
        while rows.peek().is_some() {
            let (mut batch, hooks) = split(rows.by_ref().take(chunk_size).collect());
            let outcomes = in_batch(&ctx, scope, move |ctx| async move {
                let writes: Vec<(Value, FieldMap)> = batch
                    .iter_mut()
                    .map(|(id, changeset)| {
                        let fields = changeset.take_fields();
                        (id.clone(), changeset.changes(fields))
                    })
                    .collect();
                let stored = if action_def.has_optimistic_lock() {
                    // Each record as it was read, under the action's lock.
                    let locks: Vec<_> =
                        batch.iter().map(|(_, changeset)| changeset.existing().and_then(|existing| crate::engine::lock_of(action_def, existing))).collect();
                    let mut stored = Vec::with_capacity(writes.len());
                    for ((id, changes), lock) in writes.into_iter().zip(locks) {
                        stored.push(crate::engine::update_guarded(&ctx, &R::DEF, id, changes, lock).await);
                    }
                    stored
                } else {
                    ctx.data.bulk_update(&R::DEF, ctx.tenant.as_deref(), writes).await?
                };
                finish_rows(&ctx, batch, stored, notify, scope.transactional()).await
            })
            .await;
            settle(&mut result, scope, opts.return_records, hooks, outcomes)?;
        }
        Ok(result)
    })
    .await
}
