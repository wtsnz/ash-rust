use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::action::ActionKind;
use crate::error::Result;
use crate::value::{FieldMap, Value};

/// A committed event projected for one reader. Action arguments and actor attributes
/// are never part of this payload. Metadata is empty unless explicitly allowlisted.
#[derive(Clone, Debug)]
pub struct Notification {
    pub resource: &'static str,
    pub action: String,
    pub action_kind: ActionKind,
    pub id: Value,
    pub record: crate::View,
    pub previous: Option<crate::View>,
    pub tenant: Option<String>,
    pub metadata: FieldMap,
    /// Present only when a bootstrap notifier grant explicitly requests actor IDs.
    pub actor_id: Option<uuid::Uuid>,
}

impl Notification {
    pub fn get(&self, field: &str) -> Option<&Value> {
        self.record.get(field)
    }
    pub fn previous(&self, field: &str) -> Option<&Value> {
        self.previous.as_ref().and_then(|view| view.get(field))
    }
}

/// Asynchronous event consumer notified when an action commits.
pub trait Notifier: Send + Sync + Debug + 'static {
    /// Route an opaque event. The default consumer reads as anonymous in the event's
    /// tenant. Brokers may project it for their registered readers instead.
    fn notify_event<'a>(
        &'a self,
        event: &'a crate::CommittedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(notification) = event.public_notification()? {
                self.notify(&notification).await?;
            }
            Ok(())
        })
    }

    /// Handle a notification payload.
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
}

impl Notifier for Arc<dyn Notifier> {
    fn notify_event<'a>(
        &'a self,
        event: &'a crate::CommittedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        (**self).notify_event(event)
    }
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

/// Dispatch every queued notification. Used after a transaction (or Multi) commits.
pub async fn flush_queued_notifications(
    queue: &std::sync::Mutex<Vec<crate::context::QueuedNotification>>,
    context_notifiers: &[Arc<dyn Notifier>],
) {
    let queued = {
        let mut q = queue.lock().unwrap();
        std::mem::take(&mut *q)
    };
    for item in queued {
        for notifier in context_notifiers {
            let _ = notifier.notify_event(&item.event).await;
        }
        for notifier in item.resource_notifiers {
            let _ = notifier.notify_event(&item.event).await;
        }
    }
}

/// Dispatch a notification to context notifiers and resource notifiers, or buffer if an atomic transaction queue is present.
pub(crate) async fn dispatch_notification<D>(
    ctx: &crate::context::Context<D>,
    resource_def: &'static crate::resource::ResourceDef,
    event: crate::CommittedEvent,
) -> Result<()> {
    if let Some(ref queue) = ctx.notification_queue {
        queue
            .lock()
            .unwrap()
            .push(crate::context::QueuedNotification {
                event,
                resource_notifiers: resource_def.notifiers,
            });
        return Ok(());
    }

    for notifier in ctx.notifiers() {
        let _ = notifier.notify_event(&event).await;
    }
    for notifier in resource_def.notifiers {
        let _ = notifier.notify_event(&event).await;
    }
    Ok(())
}
