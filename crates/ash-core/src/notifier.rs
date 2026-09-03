use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use crate::action::ActionKind;
use crate::actor::Actor;
use crate::error::Result;
use crate::value::{FieldMap, Value};

/// Payload sent to notifiers after a resource action has committed.
#[derive(Clone, Debug)]
pub struct Notification {
    /// Name of the resource that was acted upon (e.g. `"Order"`).
    pub resource: &'static str,
    /// Name of the action that was executed (e.g. `"create"`, `"pay"`, `"cancel"`).
    pub action: String,
    /// Kind of action (`Create`, `Update`, or `Destroy`).
    pub action_kind: ActionKind,
    /// Primary key of the affected record.
    pub id: Uuid,
    /// Fields of the record after the action was committed.
    pub record_fields: FieldMap,
    /// Previous fields before the action, if available (e.g. for update and destroy).
    pub previous_fields: Option<FieldMap>,
    /// Actor who performed the action, if present.
    pub actor: Option<Actor>,
    /// Additional context or action arguments passed during invocation.
    pub metadata: FieldMap,
}

impl Notification {
    /// Create a new notification payload.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resource: &'static str,
        action: impl Into<String>,
        action_kind: ActionKind,
        id: Uuid,
        record_fields: FieldMap,
        previous_fields: Option<FieldMap>,
        actor: Option<Actor>,
        metadata: FieldMap,
    ) -> Self {
        Self {
            resource,
            action: action.into(),
            action_kind,
            id,
            record_fields,
            previous_fields,
            actor,
            metadata,
        }
    }

    /// Read an attribute value from the committed record.
    pub fn get(&self, field: &str) -> Option<&Value> {
        self.record_fields.get(field)
    }

    /// Read a previous attribute value before the change was applied.
    pub fn previous(&self, field: &str) -> Option<&Value> {
        self.previous_fields.as_ref().and_then(|m| m.get(field))
    }
}

/// Asynchronous event consumer notified when an action commits.
pub trait Notifier: Send + Sync + Debug + 'static {
    /// Handle a notification payload.
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

impl Notifier for Arc<dyn Notifier> {
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        (**self).notify(notification)
    }
}

/// Helper struct for closures implementing [`Notifier`].
pub struct SyncFnNotifier<F> {
    name: &'static str,
    func: F,
}

impl<F> SyncFnNotifier<F>
where
    F: Fn(&Notification) -> Result<()> + Send + Sync + 'static,
{
    pub const fn new(name: &'static str, func: F) -> Self {
        Self { name, func }
    }
}

impl<F> Debug for SyncFnNotifier<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncFnNotifier")
            .field("name", &self.name)
            .finish()
    }
}

impl<F> Notifier for SyncFnNotifier<F>
where
    F: Fn(&Notification) -> Result<()> + Send + Sync + 'static,
{
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        let res = (self.func)(notification);
        Box::pin(std::future::ready(res))
    }
}

/// Dispatch a notification to context notifiers and resource notifiers, or buffer if an atomic transaction queue is present.
pub async fn dispatch_notification<D>(
    ctx: &crate::context::Context<D>,
    resource_def: &crate::resource::ResourceDef,
    notification: Notification,
) -> Result<()> {
    if let Some(ref queue) = ctx.notification_queue {
        queue.lock().unwrap().push(crate::context::QueuedNotification {
            notification,
            resource_notifiers: resource_def.notifiers,
        });
        return Ok(());
    }

    for notifier in &ctx.notifiers {
        let _ = notifier.notify(&notification).await;
    }
    for notifier in resource_def.notifiers {
        let _ = notifier.notify(&notification).await;
    }
    Ok(())
}
