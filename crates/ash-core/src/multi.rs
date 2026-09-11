mod bound;
mod result;
mod step;

pub use bound::BoundMulti;
pub use result::{IntoChangeset, MultiResult};
pub(crate) use step::Step;

use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;
use uuid::Uuid;

use crate::bulk::{BulkCreateOptions, BulkDestroyOptions};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{DataLayer, TransactionSupport};
use crate::error::{Error, Result};
use crate::resource::Resource;
use crate::value::FieldMap;

use step::{
    BulkCreateStep, BulkDestroyStep, CreateFromStep, CreateStep, DestroyFromStep, DestroyStep,
    InsertStep, RunAsyncStep, RunSyncStep, UpdateFromStep, UpdateStep,
};

/// A pipeline of named operations that execute atomically.
///
/// In the spirit of `Ash.Multi`, operations are defined sequentially and,
/// when executed against a transactional data layer, run within a single
/// database transaction. If any step fails, the entire transaction is rolled back.
pub struct Multi<D> {
    steps: Vec<Box<dyn Step<D>>>,
}

impl<D: DataLayer + 'static> Default for Multi<D> {
    fn default() -> Self {
        Self::new()
    }
}

impl<D: DataLayer + 'static> Multi<D> {
    pub fn new() -> Self {
        Self { steps: Vec::new() }
    }

    /// Add a create operation using a [`Changeset`], `Result<Changeset>`, or direct action builder.
    pub fn create<R: Resource>(
        mut self,
        name: impl Into<String>,
        changeset: impl IntoChangeset<R>,
    ) -> Self {
        self.steps.push(Box::new(CreateStep {
            name: name.into(),
            changeset: Some(changeset.into_changeset()),
        }));
        self
    }

    /// Add a dynamic create operation where the operation is computed from prior step results.
    pub fn create_from<R, C, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        R: Resource,
        C: IntoChangeset<R>,
        F: FnOnce(&Context<D>, &MultiResult) -> C + Send + 'static,
    {
        self.steps.push(Box::new(CreateFromStep {
            name: name.into(),
            func: Some(Box::new(move |ctx, res| func(ctx, res).into_changeset())),
            _phantom: PhantomData,
        }));
        self
    }

    /// Add an update operation using a [`Changeset`], `Result<Changeset>`, or direct action builder.
    pub fn update<R: Resource>(
        mut self,
        name: impl Into<String>,
        changeset: impl IntoChangeset<R>,
    ) -> Self {
        self.steps.push(Box::new(UpdateStep {
            name: name.into(),
            changeset: Some(changeset.into_changeset()),
        }));
        self
    }

    /// Add a dynamic update operation where the operation is computed from prior step results.
    pub fn update_from<R, C, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        R: Resource,
        C: IntoChangeset<R>,
        F: FnOnce(&Context<D>, &MultiResult) -> C + Send + 'static,
    {
        self.steps.push(Box::new(UpdateFromStep {
            name: name.into(),
            func: Some(Box::new(move |ctx, res| func(ctx, res).into_changeset())),
            _phantom: PhantomData,
        }));
        self
    }

    /// Add a destroy operation for an existing record.
    pub fn destroy<R: Resource>(
        mut self,
        name: impl Into<String>,
        action: &'static str,
        record: R,
    ) -> Self {
        self.steps.push(Box::new(DestroyStep {
            name: name.into(),
            action,
            record: Some(record),
        }));
        self
    }

    /// Add a dynamic destroy operation where the record is computed from prior step results.
    pub fn destroy_from<R, F>(
        mut self,
        name: impl Into<String>,
        action: &'static str,
        func: F,
    ) -> Self
    where
        R: Resource,
        F: FnOnce(&Context<D>, &MultiResult) -> Result<R> + Send + 'static,
    {
        self.steps.push(Box::new(DestroyFromStep {
            name: name.into(),
            action,
            func: Some(func),
            _phantom: PhantomData,
        }));
        self
    }

    /// Insert an arbitrary value directly into the result map under `name`.
    pub fn insert<T: Send + Sync + 'static>(mut self, name: impl Into<String>, value: T) -> Self {
        self.steps.push(Box::new(InsertStep {
            name: name.into(),
            value: Some(value),
        }));
        self
    }

    /// Add a synchronous custom function step.
    pub fn run_fn<T, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        T: Send + Sync + 'static,
        F: FnOnce(&Context<D>, &MultiResult) -> Result<T> + Send + 'static,
    {
        self.steps.push(Box::new(RunSyncStep {
            name: name.into(),
            func: Some(func),
            _phantom: PhantomData,
        }));
        self
    }

    /// Add an asynchronous custom function step.
    pub fn run<T, F, Fut>(mut self, name: impl Into<String>, func: F) -> Self
    where
        T: Send + Sync + 'static,
        F: FnOnce(Context<D>, MultiResult) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        let name_str = name.into();
        self.steps.push(Box::new(RunAsyncStep {
            name: name_str,
            func: Some(Box::new(move |ctx: Context<D>, results: MultiResult| {
                Box::pin(async move {
                    let val = func(ctx, results).await?;
                    Ok(Arc::new(val) as Arc<dyn std::any::Any + Send + Sync>)
                })
                    as std::pin::Pin<
                        Box<
                            dyn Future<Output = Result<Arc<dyn std::any::Any + Send + Sync>>>
                                + Send,
                        >,
                    >
            })),
            _phantom: PhantomData,
        }));
        self
    }

    /// Add a bulk create operation.
    pub fn bulk_create<R: Resource, I, F>(
        mut self,
        name: impl Into<String>,
        action: &'static str,
        inputs: I,
        opts: BulkCreateOptions,
    ) -> Self
    where
        I: IntoIterator<Item = F> + Send + 'static,
        F: IntoFieldMap,
    {
        let field_maps: Vec<FieldMap> = inputs
            .into_iter()
            .map(IntoFieldMap::into_field_map)
            .collect();
        self.steps.push(Box::new(BulkCreateStep::<R> {
            name: name.into(),
            action,
            inputs: field_maps,
            opts,
            _phantom: PhantomData,
        }));
        self
    }

    /// Add a bulk destroy operation.
    pub fn bulk_destroy<R: Resource>(
        mut self,
        name: impl Into<String>,
        action: &'static str,
        ids: impl IntoIterator<Item = Uuid>,
        opts: BulkDestroyOptions,
    ) -> Self {
        self.steps.push(Box::new(BulkDestroyStep::<R> {
            name: name.into(),
            action,
            ids: ids.into_iter().collect(),
            opts,
            _phantom: PhantomData,
        }));
        self
    }

    /// Internal execution of all steps sequentially without wrapping in an outer transaction.
    pub async fn run_pipeline(&mut self, ctx: &Context<D>) -> Result<MultiResult> {
        let mut results = MultiResult::new();
        for step in &mut self.steps {
            let step_name = step.name().to_string();
            if let Err(source) = step.execute(ctx, &mut results).await {
                return Err(Error::Multi {
                    step: step_name,
                    source: Box::new(source),
                });
            }
        }
        Ok(results)
    }

    /// Execute the pipeline without starting a transaction.
    pub async fn execute_without_transaction(mut self, ctx: &Context<D>) -> Result<MultiResult> {
        let (buffered_ctx, queue) = ctx.with_notification_buffer();
        let result = self.run_pipeline(&buffered_ctx).await?;

        crate::notifier::flush_queued_notifications(&queue, &ctx.notifiers).await;

        Ok(result)
    }
}

impl<D: TransactionSupport + 'static> Multi<D> {
    /// Execute the multi pipeline inside a transaction.
    ///
    /// If any step fails, the entire transaction is rolled back, queued notifications
    /// are discarded, and `Error::Multi` is returned indicating which step failed.
    ///
    /// If and only if all steps succeed and the transaction commits, all accumulated
    /// notifications are dispatched sequentially.
    pub async fn execute(mut self, ctx: &Context<D>) -> Result<MultiResult> {
        let (buffered_ctx, queue) = ctx.with_notification_buffer();
        let queue_clone = Arc::clone(&queue);
        let result = buffered_ctx
            .transaction(move |tx_ctx| {
                let tx_buffered =
                    tx_ctx.with_existing_notification_buffer(Arc::clone(&queue_clone));
                async move { self.run_pipeline(&tx_buffered).await }
            })
            .await?;

        crate::notifier::flush_queued_notifications(&queue, &ctx.notifiers).await;

        Ok(result)
    }

    /// Alias for [`execute`](Self::execute) in the spirit of `Changeset::commit`.
    pub async fn commit(self, ctx: &Context<D>) -> Result<MultiResult> {
        self.execute(ctx).await
    }
}
