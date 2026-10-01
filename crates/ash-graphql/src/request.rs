//! The Ash context a GraphQL request runs as.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::marker::PhantomData;
use std::sync::Arc;

use ash_core::{Actor, Context, DataLayer};
use async_graphql::dataloader::DataLoader;
use async_graphql::dynamic::ResolverContext;
use async_graphql::extensions::{
    Extension, ExtensionContext, ExtensionFactory, NextPrepareRequest,
};
use async_graphql::{Request, ServerResult};

use crate::dataloader::AshBatchLoader;

/// `context`, acting as `actor` when it carries no actor of its own.
fn acting_as<'a, D>(context: &'a Context<D>, actor: Option<&Actor>) -> Cow<'a, Context<D>> {
    match (&context.actor, actor) {
        (None, Some(actor)) => Cow::Owned(context.with_actor(actor.clone())),
        _ => Cow::Borrowed(context),
    }
}

/// The request's `Context<D>`, acting as an `Actor` given alongside it when the
/// context carries none. Every resolver runs as this context, so lists, gets,
/// relationships, field redaction and subscriptions agree on the actor and tenant.
pub(crate) fn request_context<'a, D: Send + Sync + 'static>(
    ctx: &ResolverContext<'a>,
) -> async_graphql::Result<Cow<'a, Context<D>>> {
    let context = ctx.ctx.data::<Context<D>>()?;
    Ok(acting_as(context, ctx.ctx.data_opt::<Actor>()))
}

/// The actor the request reads as: its context's, or an `Actor` on its own for a
/// schema run without a `Context<D>`.
pub(crate) fn request_actor<'a, D: Send + Sync + 'static>(
    ctx: &ResolverContext<'a>,
) -> Option<&'a Actor> {
    ctx.ctx
        .data_opt::<Context<D>>()
        .and_then(|context| context.actor.as_ref())
        .or_else(|| ctx.ctx.data_opt::<Actor>())
}

/// Gives each request its own [`AshBatchLoader`], bound to the context that request
/// runs as, so batched relationship loads see what the request may see.
pub(crate) struct RequestDataLoader<D>(PhantomData<fn() -> D>);

impl<D> RequestDataLoader<D> {
    pub(crate) fn new() -> Self {
        Self(PhantomData)
    }
}

impl<D: DataLayer + Clone + 'static> ExtensionFactory for RequestDataLoader<D> {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(Self::new())
    }
}

fn request_data<T: Any + Send + Sync>(request: &Request) -> Option<&T> {
    request
        .data
        .get(&TypeId::of::<T>())
        .and_then(|value| value.downcast_ref::<T>())
}

#[async_graphql::async_trait::async_trait]
impl<D: DataLayer + Clone + 'static> Extension for RequestDataLoader<D> {
    async fn prepare_request(
        &self,
        ctx: &ExtensionContext<'_>,
        request: Request,
        next: NextPrepareRequest<'_>,
    ) -> ServerResult<Request> {
        // A loader the caller supplied is theirs to bind.
        if request_data::<DataLoader<AshBatchLoader<D>>>(&request).is_some() {
            return next.run(ctx, request).await;
        }
        let context = request_data::<Context<D>>(&request).or_else(|| ctx.data_opt::<Context<D>>());
        let actor = request_data::<Actor>(&request).or_else(|| ctx.data_opt::<Actor>());
        let loader = context.map(|context| acting_as(context, actor).into_owned());
        let request = match loader {
            Some(context) => request.data(crate::AshGraphQL::create_dataloader(context)),
            None => request,
        };
        next.run(ctx, request).await
    }
}
