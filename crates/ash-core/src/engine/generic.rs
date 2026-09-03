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
pub async fn run<R, D, T, F, Fut>(ctx: &Context<D>, action: &str, f: F) -> Result<T>
where
    R: Resource,
    D: DataLayer,
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let action_def = action_named(&R::DEF, action)?;
    expect_kind(action_def, ActionKind::Generic)?;
    authorize_write(&R::DEF, action_def, ctx.actor.as_ref(), None)?;
    let result = f().await?;

    let notification = crate::notifier::Notification::new(
        R::DEF.name,
        action_def.name,
        ActionKind::Generic,
        Uuid::nil(),
        crate::value::FieldMap::new(),
        None,
        ctx.actor.clone(),
        ctx.metadata.clone(),
    ).with_tenant(ctx.tenant.clone());
    crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;

    Ok(result)
}
