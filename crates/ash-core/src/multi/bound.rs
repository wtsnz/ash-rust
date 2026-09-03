use std::future::Future;
use uuid::Uuid;

use crate::bulk::{BulkCreateOptions, BulkDestroyOptions};
use crate::changeset::IntoFieldMap;
use crate::context::Context;
use crate::data_layer::{DataLayer, TransactionSupport};
use crate::error::Result;
use crate::resource::Resource;

use super::result::{IntoChangeset, MultiResult};
use super::Multi;

/// A [`Multi`] pipeline bound to an execution [`Context`].
///
/// Created via `ctx.multi()`. Allows building and committing an atomic pipeline
/// in a single fluid chain without needing to pass `&ctx` again at commit time.
pub struct BoundMulti<D> {
    ctx: Context<D>,
    multi: Multi<D>,
}

impl<D: DataLayer + 'static> BoundMulti<D> {
    pub fn new(ctx: Context<D>) -> Self {
        Self {
            ctx,
            multi: Multi::new(),
        }
    }

    /// Add a create operation using a [`Changeset`](crate::changeset::Changeset), `Result<Changeset>`, or direct action builder.
    pub fn create<R: Resource>(mut self, name: impl Into<String>, op: impl IntoChangeset<R>) -> Self {
        self.multi = self.multi.create(name, op);
        self
    }

    /// Add a dynamic create operation where the operation is computed from prior step results.
    pub fn create_from<R, C, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        R: Resource,
        C: IntoChangeset<R>,
        F: FnOnce(&Context<D>, &MultiResult) -> C + Send + 'static,
    {
        self.multi = self.multi.create_from(name, func);
        self
    }

    /// Add an update operation using a [`Changeset`](crate::changeset::Changeset), `Result<Changeset>`, or direct action builder.
    pub fn update<R: Resource>(mut self, name: impl Into<String>, op: impl IntoChangeset<R>) -> Self {
        self.multi = self.multi.update(name, op);
        self
    }

    /// Add a dynamic update operation where the operation is computed from prior step results.
    pub fn update_from<R, C, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        R: Resource,
        C: IntoChangeset<R>,
        F: FnOnce(&Context<D>, &MultiResult) -> C + Send + 'static,
    {
        self.multi = self.multi.update_from(name, func);
        self
    }

    /// Add a destroy operation for an existing record.
    pub fn destroy<R: Resource>(
        mut self,
        name: impl Into<String>,
        action: &'static str,
        record: R,
    ) -> Self {
        self.multi = self.multi.destroy(name, action, record);
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
        self.multi = self.multi.destroy_from(name, action, func);
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
        self.multi = self.multi.bulk_create::<R, I, F>(name, action, inputs, opts);
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
        self.multi = self.multi.bulk_destroy::<R>(name, action, ids, opts);
        self
    }

    /// Insert an arbitrary value directly into the result map under `name`.
    pub fn insert<T: Send + Sync + 'static>(mut self, name: impl Into<String>, value: T) -> Self {
        self.multi = self.multi.insert(name, value);
        self
    }

    /// Add a synchronous custom function step.
    pub fn run_fn<T, F>(mut self, name: impl Into<String>, func: F) -> Self
    where
        T: Send + Sync + 'static,
        F: FnOnce(&Context<D>, &MultiResult) -> Result<T> + Send + 'static,
    {
        self.multi = self.multi.run_fn(name, func);
        self
    }

    /// Add an asynchronous custom function step.
    pub fn run<T, F, Fut>(mut self, name: impl Into<String>, func: F) -> Self
    where
        T: Send + Sync + 'static,
        F: FnOnce(Context<D>, MultiResult) -> Fut + Send + 'static,
        Fut: Future<Output = Result<T>> + Send + 'static,
    {
        self.multi = self.multi.run(name, func);
        self
    }

    /// Unbind and return the underlying [`Multi`] pipeline.
    pub fn into_multi(self) -> Multi<D> {
        self.multi
    }

    /// Commit the bound pipeline without wrapping in a transaction.
    pub async fn commit_without_transaction(self) -> Result<MultiResult> {
        self.multi.execute_without_transaction(&self.ctx).await
    }
}

impl<D: TransactionSupport + 'static> BoundMulti<D> {
    /// Commit the bound pipeline atomically inside a database transaction.
    pub async fn commit(self) -> Result<MultiResult> {
        self.multi.execute(&self.ctx).await
    }
}
