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
    authority: Arc<RuntimeAuthority>,
    system: bool,
    cursor_key: Arc<crate::cursor::CursorKey>,
    notifiers: Vec<Arc<dyn Notifier>>,
    notification_queue: Option<Arc<Mutex<Vec<QueuedNotification>>>>,
}

impl<D> Clone for Runtime<D> {
    fn clone(&self) -> Self {
        Self {
            data: Arc::clone(&self.data),
            authority: Arc::clone(&self.authority),
            system: self.system,
            cursor_key: Arc::clone(&self.cursor_key),
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
            authority: Arc::new(RuntimeAuthority),
            system: false,
            cursor_key: Arc::new(crate::cursor::CursorKey::random()),
            notifiers: Vec::new(),
            notification_queue: None,
        }
    }

    /// Configure a shared production cursor key before binding contexts. Replacing the
    /// key invalidates earlier cursors. The default is random and Runtime-local.
    pub fn with_cursor_key(mut self, key: [u8; 32]) -> Self {
        self.cursor_key = Arc::new(crate::cursor::CursorKey::new(key));
        self
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
                authority: Arc::clone(&self.authority),
                actor,
                tenant,
                authorization,
                cursor_key: Arc::clone(&self.cursor_key),
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

    /// Grant mutation of explicit credential attributes on one resource.
    pub fn credential_writes<R: crate::Resource>(
        &self,
        fields: impl IntoIterator<Item = &'static str>,
    ) -> crate::CredentialWritePermit {
        crate::CredentialWritePermit {
            resource: &R::DEF,
            fields: fields.into_iter().collect(),
            authority: Arc::clone(&self.authority),
        }
    }

    /// Grant credential reads of one resource. The grant never bypasses tenancy.
    pub fn credential_reads<R: crate::Resource>(&self) -> crate::CredentialReadPermit {
        crate::CredentialReadPermit {
            resource: &R::DEF,
            authority: Arc::clone(&self.authority),
        }
    }
}

#[derive(Debug)]
pub(crate) struct RuntimeAuthority;

#[derive(Debug)]
pub(crate) struct Scope {
    pub(crate) authority: Arc<RuntimeAuthority>,
    actor: Option<Actor>,
    tenant: Option<String>,
    authorization: Authorization,
    pub(crate) cursor_key: Arc<crate::cursor::CursorKey>,
}

#[derive(Debug)]
enum Authorization {
    Enforce,
    System(Actor),
}

impl Scope {
    pub(crate) fn actor(&self) -> Option<&Actor> {
        self.actor.as_ref()
    }
    pub(crate) fn tenant(&self) -> Option<&str> {
        self.tenant.as_deref()
    }
}

/// An execution handle with immutable actor and tenant identity.
///
/// Schema installation requires startup Runtime authority, not a request Context.
///
/// ```compile_fail,E0599
/// async fn install<D: ash_core::SchemaSupport>(ctx: &ash_core::Context<D>) {
///     ctx.install(&[]).await;
/// }
/// ```
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
///
/// ```compile_fail
/// let ctx = ash_core::Runtime::new(()).bootstrap();
/// let backend = &ctx.data;
/// ```
#[derive(Debug)]
pub struct Context<D> {
    pub(crate) scope: Arc<Scope>,
    metadata: FieldMap,
    data: Arc<D>,
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

    /// Policy-compilation identity carried by executable read plans. System scopes
    /// retain sealed authorization here. Changes, validations, hooks and notifications
    /// use `actor()` for execution identity.
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

impl<D: SchemaSupport> Runtime<D> {
    /// Install startup schemas and resource SQL statements with server-owned authority.
    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        self.data.install_resources(resources).await
    }
}

impl<D: DataLayer> Context<D> {
    /// Multi-record actions require a real transaction, including inside notification buffers.
    pub(crate) async fn atomically<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(Context<D>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        let queue = Arc::new(Mutex::new(Vec::new()));
        let outside = self.clone();
        let tx_queue = Arc::clone(&queue);
        let result = self
            .data
            .in_transaction(move |tx_data| match tx_data {
                Some(data) => futures_util::future::Either::Left(f(Context {
                    data: Arc::new(data),
                    notification_queue: Some(tx_queue),
                    ..outside
                })),
                None => futures_util::future::Either::Right(std::future::ready(Err(
                    crate::Error::AtomicityUnsupported,
                ))),
            })
            .await;
        if result.is_ok() {
            if let Some(outer) = &self.notification_queue {
                outer
                    .lock()
                    .unwrap()
                    .extend(queue.lock().unwrap().drain(..));
            } else {
                crate::notifier::flush_queued_notifications(&queue, &self.notifiers).await;
            }
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
        let authority = Arc::clone(&self.authority);
        let cursor_key = Arc::clone(&self.cursor_key);
        let created_queue = self.notification_queue.is_none();
        let queue = self.notification_queue.clone().unwrap_or_default();
        let tx_queue = Arc::clone(&queue);
        let result = self
            .data
            .transaction(move |data| {
                f(Runtime {
                    data: Arc::new(data.clone()),
                    authority,
                    system,
                    cursor_key,
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
