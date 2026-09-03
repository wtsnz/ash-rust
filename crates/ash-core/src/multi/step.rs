use std::any::Any;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use crate::bulk::{BulkCreateOptions, BulkDestroyOptions};
use crate::changeset::Changeset;
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::{Error, Result};
use crate::resource::Resource;
use crate::value::FieldMap;

use super::result::MultiResult;

pub trait Step<D>: Send {
    fn name(&self) -> &str;
    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

pub(crate) struct CreateStep<R: Resource> {
    pub(crate) name: String,
    pub(crate) changeset: Option<Result<Changeset<R>>>,
}

impl<D: DataLayer, R: Resource> Step<D> for CreateStep<R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let cs_res = self.changeset.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let cs = cs_res?;
            let record = cs.commit(ctx).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

#[allow(clippy::type_complexity)]
pub(crate) struct CreateFromStep<D, R: Resource> {
    pub(crate) name: String,
    pub(crate) func: Option<Box<dyn FnOnce(&Context<D>, &MultiResult) -> Result<Changeset<R>> + Send>>,
    pub(crate) _phantom: PhantomData<fn(&Context<D>) -> R>,
}

impl<D: DataLayer, R: Resource> Step<D> for CreateFromStep<D, R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let f = self.func.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let cs = f(ctx, results)?;
            let record = cs.commit(ctx).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

pub(crate) struct UpdateStep<R: Resource> {
    pub(crate) name: String,
    pub(crate) changeset: Option<Result<Changeset<R>>>,
}

impl<D: DataLayer, R: Resource> Step<D> for UpdateStep<R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let cs_res = self.changeset.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let cs = cs_res?;
            let record = cs.commit(ctx).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

#[allow(clippy::type_complexity)]
pub(crate) struct UpdateFromStep<D, R: Resource> {
    pub(crate) name: String,
    pub(crate) func: Option<Box<dyn FnOnce(&Context<D>, &MultiResult) -> Result<Changeset<R>> + Send>>,
    pub(crate) _phantom: PhantomData<fn(&Context<D>) -> R>,
}

impl<D: DataLayer, R: Resource> Step<D> for UpdateFromStep<D, R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let f = self.func.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let cs = f(ctx, results)?;
            let record = cs.commit(ctx).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

pub(crate) struct DestroyStep<R: Resource> {
    pub(crate) name: String,
    pub(crate) action: &'static str,
    pub(crate) record: Option<R>,
}

impl<D: DataLayer, R: Resource> Step<D> for DestroyStep<R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let record = self.record.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            crate::engine::destroy_existing(ctx, self.action, record.clone()).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

pub(crate) struct DestroyFromStep<D, R, F> {
    pub(crate) name: String,
    pub(crate) action: &'static str,
    pub(crate) func: Option<F>,
    pub(crate) _phantom: PhantomData<fn(&Context<D>) -> R>,
}

impl<D: DataLayer, R: Resource, F> Step<D> for DestroyFromStep<D, R, F>
where
    F: FnOnce(&Context<D>, &MultiResult) -> Result<R> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let f = self.func.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let record = f(ctx, results)?;
            crate::engine::destroy_existing(ctx, self.action, record.clone()).await?;
            results.insert(self.name.clone(), record);
            Ok(())
        })
    }
}

pub(crate) struct InsertStep<T> {
    pub(crate) name: String,
    pub(crate) value: Option<T>,
}

impl<D: DataLayer, T: Send + Sync + 'static> Step<D> for InsertStep<T> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        _ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let value = self.value.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            results.insert(self.name.clone(), value);
            Ok(())
        })
    }
}

pub(crate) struct RunSyncStep<D, T, F> {
    pub(crate) name: String,
    pub(crate) func: Option<F>,
    pub(crate) _phantom: PhantomData<fn(&Context<D>) -> T>,
}

impl<D: DataLayer, T: Send + Sync + 'static, F> Step<D> for RunSyncStep<D, T, F>
where
    F: FnOnce(&Context<D>, &MultiResult) -> Result<T> + Send + 'static,
{
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let f = self.func.take().ok_or_else(|| {
                Error::Invalid(format!("step `{}` has already been executed", self.name))
            })?;
            let value = f(ctx, results)?;
            results.insert(self.name.clone(), value);
            Ok(())
        })
    }
}

#[allow(clippy::type_complexity)]
pub(crate) struct RunAsyncStep<D> {
    pub(crate) name: String,
    pub(crate) func: Option<
        Box<
            dyn FnOnce(
                    Context<D>,
                    MultiResult,
                ) -> Pin<
                    Box<dyn Future<Output = Result<Arc<dyn Any + Send + Sync>>> + Send>,
                > + Send,
        >,
    >,
    pub(crate) _phantom: PhantomData<fn(&Context<D>)>,
}

impl<D: DataLayer + 'static> Step<D> for RunAsyncStep<D> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let f = self.func.take().unwrap();
        let ctx = ctx.clone();
        let res_clone = results.clone();
        Box::pin(async move {
            let val = f(ctx, res_clone).await?;
            results.insert_arc(self.name.clone(), val);
            Ok(())
        })
    }
}

pub(crate) struct BulkCreateStep<R: Resource> {
    pub(crate) name: String,
    pub(crate) action: &'static str,
    pub(crate) inputs: Vec<FieldMap>,
    pub(crate) opts: BulkCreateOptions,
    pub(crate) _phantom: PhantomData<R>,
}

impl<D: DataLayer, R: Resource> Step<D> for BulkCreateStep<R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let inputs = std::mem::take(&mut self.inputs);
            let res = crate::bulk::bulk_create::<R, D, _, _>(ctx, self.action, inputs, self.opts.clone()).await?;
            results.insert(self.name.clone(), res);
            Ok(())
        })
    }
}

pub(crate) struct BulkDestroyStep<R: Resource> {
    pub(crate) name: String,
    pub(crate) action: &'static str,
    pub(crate) ids: Vec<Uuid>,
    pub(crate) opts: BulkDestroyOptions,
    pub(crate) _phantom: PhantomData<R>,
}

impl<D: DataLayer, R: Resource> Step<D> for BulkDestroyStep<R> {
    fn name(&self) -> &str {
        &self.name
    }

    fn execute<'a>(
        &'a mut self,
        ctx: &'a Context<D>,
        results: &'a mut MultiResult,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let ids = std::mem::take(&mut self.ids);
            let res = crate::bulk::bulk_destroy::<R, D>(ctx, self.action, &ids, self.opts.clone()).await?;
            results.insert(self.name.clone(), res);
            Ok(())
        })
    }
}
