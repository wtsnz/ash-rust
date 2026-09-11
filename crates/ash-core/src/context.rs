use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::actor::Actor;
use crate::data_layer::{SchemaSupport, TransactionSupport};
use crate::error::Result;
use crate::notifier::{Notification, Notifier};
use crate::resource::ResourceDef;
use crate::value::{FieldMap, Value};

/// Notification queued during an atomic transaction buffer.
#[derive(Clone, Debug)]
pub struct QueuedNotification {
    pub notification: Notification,
    pub resource_notifiers: &'static [&'static dyn Notifier],
}

#[derive(Debug)]
pub struct Context<D> {
    pub actor: Option<Actor>,
    pub tenant: Option<String>,
    pub metadata: FieldMap,
    pub data: Arc<D>,
    pub notifiers: Vec<Arc<dyn Notifier>>,
    pub(crate) notification_queue: Option<Arc<Mutex<Vec<QueuedNotification>>>>,
}

impl<D> Clone for Context<D> {
    fn clone(&self) -> Self {
        Self {
            actor: self.actor.clone(),
            tenant: self.tenant.clone(),
            metadata: self.metadata.clone(),
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
            tenant: None,
            metadata: FieldMap::new(),
            data: Arc::new(data),
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    pub fn from_arc(data: Arc<D>) -> Self {
        Self {
            actor: None,
            tenant: None,
            metadata: FieldMap::new(),
            data,
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    pub fn with_actor(&self, actor: Actor) -> Self {
        Self {
            actor: Some(actor),
            tenant: self.tenant.clone(),
            metadata: self.metadata.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    pub fn without_actor(&self) -> Self {
        Self {
            actor: None,
            tenant: self.tenant.clone(),
            metadata: self.metadata.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Set the tenant for this execution context.
    pub fn with_tenant(&self, tenant: impl Into<String>) -> Self {
        Self {
            actor: self.actor.clone(),
            tenant: Some(tenant.into()),
            metadata: self.metadata.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Clear the tenant on this execution context.
    pub fn without_tenant(&self) -> Self {
        Self {
            actor: self.actor.clone(),
            tenant: None,
            metadata: self.metadata.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Get the current tenant, if set.
    pub fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }

    /// Set a metadata entry on this execution context.
    pub fn with_metadata(&self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        let mut metadata = self.metadata.clone();
        metadata.insert(key.into(), value.into());
        Self {
            actor: self.actor.clone(),
            tenant: self.tenant.clone(),
            metadata,
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Merge additional metadata entries into this execution context.
    pub fn with_all_metadata(&self, extra: FieldMap) -> Self {
        let mut metadata = self.metadata.clone();
        metadata.extend(extra);
        Self {
            actor: self.actor.clone(),
            tenant: self.tenant.clone(),
            metadata,
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Access the metadata bag on this execution context.
    pub fn metadata(&self) -> &FieldMap {
        &self.metadata
    }

    /// Retrieve a specific metadata value by key.
    pub fn get_metadata(&self, key: &str) -> Option<&Value> {
        self.metadata.get(key)
    }

    /// Attach a [`Notifier`] to this execution context.
    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifiers.push(notifier);
        self
    }

    /// Attach multiple [`Notifier`]s to this execution context.
    pub fn with_notifiers(
        mut self,
        notifiers: impl IntoIterator<Item = Arc<dyn Notifier>>,
    ) -> Self {
        self.notifiers.extend(notifiers);
        self
    }

    /// Create a child context that queues notifications in memory instead of immediately dispatching them.
    pub fn with_notification_buffer(&self) -> (Self, Arc<Mutex<Vec<QueuedNotification>>>) {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let ctx = Self {
            actor: self.actor.clone(),
            tenant: self.tenant.clone(),
            metadata: self.metadata.clone(),
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
            tenant: self.tenant.clone(),
            metadata: self.metadata.clone(),
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
        let tenant = self.tenant.clone();
        let metadata = self.metadata.clone();
        let notifiers = self.notifiers.clone();
        let created_queue = self.notification_queue.is_none();
        let queue = self
            .notification_queue
            .clone()
            .unwrap_or_else(|| Arc::new(Mutex::new(Vec::new())));
        let tx_queue = Arc::clone(&queue);
        let result = self
            .data
            .transaction(move |tx_data| {
                let tx_ctx = Context {
                    actor,
                    tenant,
                    metadata,
                    data: Arc::new(tx_data.clone()),
                    notifiers,
                    notification_queue: Some(tx_queue),
                };
                f(tx_ctx)
            })
            .await;

        if created_queue {
            match &result {
                Ok(_) => crate::notifier::flush_queued_notifications(&queue, &self.notifiers).await,
                Err(_) => {
                    queue.lock().unwrap().clear();
                }
            }
        }

        result
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
