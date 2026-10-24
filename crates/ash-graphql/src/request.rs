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

/// The request's bound context. Identity is never filled from separate request data.
pub(crate) fn request_context<'a, D: Send + Sync + 'static>(
    ctx: &ResolverContext<'a>,
) -> async_graphql::Result<Cow<'a, Context<D>>> {
    let context = ctx.ctx.data::<Context<D>>()?;
    Ok(Cow::Borrowed(context))
}

/// The policy identity of the bound scope, including explicit system authority.
pub(crate) fn request_actor<'a, D: Send + Sync + 'static>(
    ctx: &ResolverContext<'a>,
) -> Option<&'a Actor> {
    ctx.ctx
        .data_opt::<Context<D>>()
        .and_then(|context| context.authorization_actor())
}

/// Network adapters require request-local binding, even when the schema has a default.
pub(crate) struct RequireBoundContext;

/// Gives each request its own [`AshBatchLoader`], bound to the context that request
/// runs as, so batched relationship loads see what the request may see.
pub(crate) struct RequestDataLoader<D> {
    enabled: bool,
    marker: PhantomData<fn() -> D>,
}

impl<D> RequestDataLoader<D> {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            marker: PhantomData,
        }
    }
}

impl<D: DataLayer + Clone + 'static> ExtensionFactory for RequestDataLoader<D> {
    fn create(&self) -> Arc<dyn Extension> {
        Arc::new(Self::new(self.enabled))
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
        if request_data::<RequireBoundContext>(&request).is_some()
            && request_data::<Context<D>>(&request).is_none()
        {
            return Err(async_graphql::ServerError::new(
                "request context was not bound by the server adapter",
                None,
            ));
        }
        if !self.enabled {
            return next.run(ctx, request).await;
        }
        // A loader the caller supplied is theirs to bind.
        if request_data::<DataLoader<AshBatchLoader<D>>>(&request).is_some() {
            return next.run(ctx, request).await;
        }
        let context = request_data::<Context<D>>(&request).or_else(|| ctx.data_opt::<Context<D>>());
        let loader = context.cloned();
        let request = match loader {
            Some(context) => request.data(crate::AshGraphQL::create_dataloader(context)),
            None => request,
        };
        next.run(ctx, request).await
    }
}
