// Context owns the deprecated compatibility field until executable plans replace raw data access.
#![allow(deprecated)]

use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::actor::Actor;
use crate::data_layer::{DataLayer, SchemaSupport, TransactionSupport};
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

/// Server-owned authority for binding request identity and issuing bootstrap grants.
/// Keep this at the application boundary; pass bound contexts to execution code.
#[derive(Debug)]
pub struct Runtime<D> {
    data: Arc<D>,
    system: bool,
    notifiers: Vec<Arc<dyn Notifier>>,
    notification_queue: Option<Arc<Mutex<Vec<QueuedNotification>>>>,
}

impl<D> Clone for Runtime<D> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            system: self.system,
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }
}

impl<D> Runtime<D> {
    pub fn new(data: D) -> Self {
        Self::from_arc(Arc::new(data))
    }

    pub fn from_arc(data: Arc<D>) -> Self {
        Self {
            data,
            system: false,
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    pub fn with_notifier(mut self, notifier: Arc<dyn Notifier>) -> Self {
        self.notifiers.push(notifier);
        self
    }

    /// Bind identity resolved by trusted application code. Each call owns a new scope.
    pub fn bind(&self, actor: Option<Actor>, tenant: Option<String>) -> Context<D> {
        let actor = actor.map(Actor::into_request_actor);
        let authorization = if self.system {
            Authorization::System(actor.clone().unwrap_or_else(Actor::system).into_system())
        } else {
            Authorization::Enforce
        };
        Context {
            scope: Arc::new(Scope {
                actor,
                tenant,
                authorization,
            }),
            metadata: FieldMap::new(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }

    /// Privileged bootstrap authority. Bind each operation to an explicit tenant.
    /// Authorization is bypassed; tenancy remains enforced, including required tenants.
    pub fn system(&self) -> Self {
        Self {
            system: true,
            ..self.clone()
        }
    }

    /// Deliberately create an anonymous, tenantless context during bootstrap.
    pub fn bootstrap(&self) -> Context<D> {
        self.bind(None, None)
    }

    /// Grant credential reads of one resource. The grant never bypasses tenancy.
    pub fn credential_reads<R: crate::Resource>(&self) -> crate::CredentialReadPermit {
        crate::CredentialReadPermit::new(&R::DEF)
    }
}

#[derive(Debug)]
pub(crate) struct Scope {
    actor: Option<Actor>,
    tenant: Option<String>,
    authorization: Authorization,
}

#[derive(Debug)]
enum Authorization {
    Enforce,
    System(Actor),
}

impl Scope {
    pub(crate) fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
}

/// An execution handle with immutable actor and tenant identity.
///
/// ```compile_fail
/// let runtime = ash_core::Runtime::new(());
/// let mut ctx = runtime.bootstrap();
/// ctx.actor = None;
/// ```
///
/// ```compile_fail
/// let ctx = ash_core::Runtime::new(()).bootstrap();
/// ctx.with_tenant("another-tenant");
/// ```
#[derive(Debug)]
pub struct Context<D> {
    pub(crate) scope: Arc<Scope>,
    metadata: FieldMap,
    #[deprecated(
        note = "raw data access bypasses scoped execution; use Context::access and action builders"
    )]
    pub data: Arc<D>,
    notifiers: Vec<Arc<dyn Notifier>>,
    pub(crate) notification_queue: Option<Arc<Mutex<Vec<QueuedNotification>>>>,
}

impl<D> Clone for Context<D> {
    fn clone(&self) -> Self {
        Self {
            scope: Arc::clone(&self.scope),
            metadata: self.metadata.clone(),
            data: Arc::clone(&self.data),
            notifiers: self.notifiers.clone(),
            notification_queue: self.notification_queue.clone(),
        }
    }
}

impl<D> Context<D> {
    pub(crate) fn raw_data(&self) -> &D {
        &self.data
    }

    pub fn actor(&self) -> Option<&Actor> {
        self.scope.actor.as_ref()
    }

    /// Policy-adapter bridge until executable plans own authorization. This carries
    /// sealed system authority for policy compilation only. Changes, validations,
    /// hooks and notifications must use `actor()` for execution identity.
    pub fn authorization_actor(&self) -> Option<&Actor> {
        match &self.scope.authorization {
            Authorization::Enforce => self.actor(),
            Authorization::System(actor) => Some(actor),
        }
    }

    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant.as_deref()
    }

    pub fn notifiers(&self) -> &[Arc<dyn Notifier>] {
        &self.notifiers
    }

    /// Whether a request loader can serve this exact scope and backend handle.
    pub fn same_execution(&self, other: &Self) -> bool {
        self.same_scope(other) && Arc::ptr_eq(&self.data, &other.data)
    }

    /// Whether these handles share the exact request scope, including transactions.
    pub fn same_scope<E>(&self, other: &Context<E>) -> bool {
        Arc::ptr_eq(&self.scope, &other.scope)
    }

    /// Set a metadata entry on this execution context.
    pub fn with_metadata(&self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        let mut metadata = self.metadata.clone();
        metadata.insert(key.into(), value.into());
        Self {
            scope: Arc::clone(&self.scope),
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
            scope: Arc::clone(&self.scope),
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
            scope: Arc::clone(&self.scope),
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
            scope: Arc::clone(&self.scope),
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

impl<D: DataLayer> Context<D> {
    /// Runs `f` in one transaction where the data layer has them
    /// ([`DataLayer::in_transaction`]), its notifications sent once it commits, as Ash
    /// runs an action in a transaction; within one already, or where the data layer has
    /// none, as it is.
    pub(crate) async fn atomically<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Context<D>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        if self.notification_queue.is_some() {
            return f(self.clone()).await;
        }
        let queue = Arc::new(Mutex::new(Vec::new()));
        let outside = self.clone();
        let tx_queue = Arc::clone(&queue);
        let result = self
            .data
            .in_transaction(move |tx_data| {
                let ctx = match tx_data {
                    Some(tx_data) => Context {
                        data: Arc::new(tx_data),
                        notification_queue: Some(tx_queue),
                        ..outside
                    },
                    None => outside,
                };
                f(ctx)
            })
            .await;
        match &result {
            Ok(_) => crate::notifier::flush_queued_notifications(&queue, &self.notifiers).await,
            Err(_) => queue.lock().unwrap().clear(),
        }
        result
    }
}

impl<D: TransactionSupport> Context<D> {
    pub async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Context<D>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let scope = Arc::clone(&self.scope);
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
                    scope,
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

impl<D: TransactionSupport> Runtime<D> {
    /// Run several explicitly bound scopes on the same transaction backend.
    /// Prepare changesets inside this closure in the scope that will commit them.
    pub async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Runtime<D>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let notifiers = self.notifiers.clone();
        let system = self.system;
        let created_queue = self.notification_queue.is_none();
        let queue = self.notification_queue.clone().unwrap_or_default();
        let tx_queue = Arc::clone(&queue);
        let result = self
            .data
            .transaction(move |data| {
                f(Runtime {
                    data: Arc::new(data.clone()),
                    system,
                    notifiers,
                    notification_queue: Some(tx_queue),
                })
            })
            .await;
        if created_queue {
            match &result {
                Ok(_) => crate::notifier::flush_queued_notifications(&queue, &self.notifiers).await,
                Err(_) => queue.lock().unwrap().clear(),
            }
        }
        result
    }
}
