use std::future::Future;
use uuid::Uuid;

use crate::action::ActionKind;
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::Result;
use crate::pipeline::{action_named, expect_kind};
use crate::policy::authorize_write;
use crate::resource::Resource;

/// Generic action: policies run, then `f`. No persist.
pub async fn run<R, D, T, F, Fut>(
    ctx: &Context<D>,
    action: &str,
    f: F,
) -> Result<crate::returned::ActionResult<T>>
where
    R: Resource,
    D: DataLayer,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<crate::returned::ActionOutput<T>>>,
{
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Generic)?;
    authorize_write(&R::DEF, action_def, ctx.authorization_actor(), None)?;
    let result = f().await?;

    let event = crate::CommittedEvent::new(
        ctx,
        &R::DEF,
        action_def.name,
        ActionKind::Generic,
        Uuid::nil().into(),
        crate::FieldMap::new(),
        None,
        ctx.metadata().clone(),
    );
    crate::notifier::dispatch_notification(ctx, &R::DEF, event).await?;

    result.present(ctx)
}
