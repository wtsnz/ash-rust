use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::actor::Actor;
use crate::data_layer::{SchemaSupport, TransactionSupport};
use crate::error::Result;
use crate::notifier::{Notification, Notifier};
use crate::resource::ResourceDef;

/// Notification queued during an atomic transaction buffer.
#[derive(Clone, Debug)]
pub struct QueuedNotification {
    pub notification: Notification,
    pub resource_notifiers: &'static [&'static dyn Notifier],
}

#[derive(Debug)]
pub struct Context<D> {
    pub actor: Option<Actor>,
    pub data: Arc<D>,
    pub notifiers: Vec<Arc<dyn Notifier>>,
    pub(crate) notification_queue: Option<Arc<Mutex<Vec<QueuedNotification>>>>,
}

impl<D> Clone for Context<D> {
    fn clone(&self) -> Self {
        Self {
            actor: self.actor.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }
}

impl<D> Context<D> {
    pub fn new(data: D) -> Self {
        Self {
            actor: None,
            data: Arc::new(data),
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    pub fn from_arc(data: Arc<D>) -> Self {
        Self {
            actor: None,
            data,
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    pub fn with_actor(&self, actor: Actor) -> Self {
        Self {
            actor: Some(actor),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    pub fn without_actor(&self) -> Self {
        Self {
            actor: None,
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Attach a [`Notifier`] to this execution context.
    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifiers.push(notifier);
        self
    }

    /// Attach multiple [`Notifier`]s to this execution context.
    pub fn with_notifiers(mut self, notifiers: impl IntoIterator<Item = Arc<dyn Notifier>>) -> Self {
        self.notifiers.extend(notifiers);
        self
    }

    /// Create a child context that queues notifications in memory instead of immediately dispatching them.
    pub fn with_notification_buffer(&self) -> (Self, Arc<Mutex<Vec<QueuedNotification>>>) {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let ctx = Self {
            actor: self.actor.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: Some(Arc::clone(&queue)),
        };
        (ctx, queue)
    }

    /// Create a context sharing an existing notification buffer queue.
    pub fn with_existing_notification_buffer(
        &self,
        queue: Arc<Mutex<Vec<QueuedNotification>>>,
    ) -> Self {
        Self {
            actor: self.actor.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: Some(queue),
        }
    }
}

impl<D: crate::data_layer::DataLayer + 'static> Context<D> {
    /// Start a fluid [`BoundMulti`](crate::multi::BoundMulti) pipeline bound to this context.
    pub fn multi(&self) -> crate::multi::BoundMulti<D> {
        crate::multi::BoundMulti::new(self.clone())
    }
}

impl<D: SchemaSupport> Context<D> {
    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        self.data.install_resources(resources).await
    }
}

impl<D: TransactionSupport> Context<D> {
    pub async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Context<D>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let actor = self.actor.clone();
        let notifiers = self.notifiers.clone();
        let notification_queue = self.notification_queue.clone();
        self.data
            .transaction(move |tx_data| {
                let tx_ctx = Context {
                    actor,
                    data: Arc::new(tx_data.clone()),
                    notifiers,
                    notification_queue,
                };
                f(tx_ctx)
            })
            .await
    }
}

impl<D: TransactionSupport + 'static> Context<D> {
    pub async fn run_multi(
        &self,
        multi: crate::multi::Multi<D>,
    ) -> Result<crate::multi::MultiResult> {
        multi.execute(self).await
    }
}
