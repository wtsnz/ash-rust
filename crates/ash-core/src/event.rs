//! Opaque committed events and reader-bound projection.

use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;

use crate::{
    ActionKind, ClientExpr, Context, Error, FieldMap, Filter, FilterMatch, MetadataDef,
    Notification, Notifier, Resource, ResourceDef, Result, Runtime, Value, View,
};

/// A retained bound scope. Authenticated scopes share only through the same binding.
/// Ordinary anonymous scopes share within one Runtime, backend and tenant.
#[derive(Clone)]
pub struct SubscriptionScope {
    scope: Arc<crate::context::Scope>,
    backend: usize,
}
impl SubscriptionScope {
    pub(crate) fn new<D>(ctx: &Context<D>, backend: usize) -> Self {
        Self {
            scope: Arc::clone(&ctx.scope),
            backend,
        }
    }
    fn public(&self) -> bool {
        self.scope.ordinary_anonymous()
    }
}
impl PartialEq for SubscriptionScope {
    fn eq(&self, other: &Self) -> bool {
        self.backend == other.backend
            && (Arc::ptr_eq(&self.scope, &other.scope)
                || (self.public()
                    && other.public()
                    && Arc::ptr_eq(&self.scope.authority, &other.scope.authority)
                    && Arc::ptr_eq(&self.scope.cursor_key, &other.scope.cursor_key)
                    && self.scope.tenant() == other.scope.tenant()))
    }
}
impl Eq for SubscriptionScope {}
impl Hash for SubscriptionScope {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.backend.hash(state);
        self.public().hash(state);
        if self.public() {
            Arc::as_ptr(&self.scope.authority).hash(state);
            Arc::as_ptr(&self.scope.cursor_key).hash(state);
            self.scope.tenant().hash(state);
        } else {
            Arc::as_ptr(&self.scope).hash(state);
        }
    }
}
impl std::fmt::Debug for SubscriptionScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubscriptionScope").finish_non_exhaustive()
    }
}

/// A private snapshot carried between core and event routers. It has no raw field,
/// actor, argument, serialization or construction API.
///
/// ```compile_fail
/// fn raw(event: ash_core::CommittedEvent) { let _ = event.record; }
/// ```
///
/// ```compile_fail
/// fn serialize(event: ash_core::CommittedEvent) { serde_json::to_string(&event).unwrap(); }
/// ```
#[derive(Clone)]
pub struct CommittedEvent {
    context: Context<()>,
    resource: &'static ResourceDef,
    action: String,
    kind: ActionKind,
    id: Value,
    record: FieldMap,
    previous: Option<FieldMap>,
    metadata: FieldMap,
}
impl std::fmt::Debug for CommittedEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommittedEvent").finish_non_exhaustive()
    }
}
impl CommittedEvent {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &str,
        kind: ActionKind,
        id: Value,
        record: FieldMap,
        previous: Option<FieldMap>,
        metadata: FieldMap,
    ) -> Self {
        Self {
            context: ctx.identity_context(),
            resource,
            action: action.into(),
            kind,
            id,
            record,
            previous,
            metadata,
        }
    }

    /// Resolve admission, filters and field projection as the registered reader.
    pub fn project(&self, reader: &EventReader) -> Result<Option<Notification>> {
        if !same_resource(self.resource, reader.resource)
            || !Arc::ptr_eq(
                &self.context.scope.authority,
                &reader.context.scope.authority,
            )
            || self.context.tenant() != reader.context.tenant()
        {
            return Ok(None);
        }
        if !crate::record_visible(
            self.resource,
            reader.context.authorization_actor(),
            reader.context.tenant(),
            &self.record,
        ) {
            return Ok(None);
        }
        if reader.predicate.as_ref().is_some_and(|predicate| {
            predicate.snapshot_match(self.resource, &self.record) != FilterMatch::Match
        }) {
            return Ok(None);
        }
        self.project_admitted(
            &reader.context,
            reader.selection.as_deref(),
            &reader.metadata,
            false,
        )
        .map(Some)
    }

    fn project_admitted(
        &self,
        ctx: &Context<()>,
        selection: Option<&[String]>,
        metadata: &[MetadataDef],
        actor_id: bool,
    ) -> Result<Notification> {
        let present = |fields: &FieldMap| -> Result<View> {
            // Hook metadata has a separate allowlist. A View must not retain it implicitly.
            let fields: FieldMap = fields
                .iter()
                .filter(|(name, _)| self.resource.attribute(name).is_some())
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect();
            ctx.present(self.resource, fields.into(), selection)
        };
        let mut allowed = FieldMap::new();
        for definition in metadata {
            if let Some(value) = self.metadata.get(definition.name) {
                allowed.insert(
                    definition.name.into(),
                    ctx.present_value(definition.ty, value.clone())?.to_value(),
                );
            }
        }
        // A previous snapshot is independently authorized, preventing owner changes
        // from exposing the previous owner's protected data.
        let previous = self
            .previous
            .as_ref()
            .filter(|fields| {
                crate::record_visible(
                    self.resource,
                    ctx.authorization_actor(),
                    ctx.tenant(),
                    fields,
                )
            })
            .map(present)
            .transpose()?;
        Ok(Notification {
            resource: self.resource.name,
            action: self.action.clone(),
            action_kind: self.kind,
            id: self.id.clone(),
            record: present(&self.record)?,
            previous,
            tenant: self.context.tenant().map(str::to_owned),
            metadata: allowed,
            actor_id: actor_id
                .then(|| self.context.actor().map(|actor| actor.id))
                .flatten(),
        })
    }

    pub(crate) fn public_notification(&self) -> Result<Option<Notification>> {
        let mut ctx = self.context.clone();
        ctx.scope = self.context.scope.anonymous();
        self.project(&ctx.event_reader(self.resource, None)?)
    }
}

/// Registration validates every snapshot predicate before any event can arrive.
#[derive(Clone)]
pub struct EventReader {
    context: Context<()>,
    resource: &'static ResourceDef,
    predicate: Option<Filter>,
    selection: Option<Vec<String>>,
    metadata: Vec<MetadataDef>,
}
impl std::fmt::Debug for EventReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventReader")
            .field("resource", &self.resource.name)
            .field("tenant", &self.tenant())
            .finish_non_exhaustive()
    }
}
impl EventReader {
    pub fn select(mut self, fields: impl IntoIterator<Item = String>) -> Self {
        self.selection = Some(fields.into_iter().collect());
        self
    }
    pub fn with_metadata(mut self, definitions: &[MetadataDef]) -> Self {
        self.metadata = definitions.to_vec();
        self
    }
    pub fn tenant(&self) -> Option<&str> {
        self.context.tenant()
    }
}
impl<D> Context<D> {
    pub fn event_reader(
        &self,
        resource: &'static ResourceDef,
        filter: Option<ClientExpr<Filter>>,
    ) -> Result<EventReader> {
        let query = crate::scope_read(
            resource,
            resource.default_read(),
            self.authorization_actor(),
            &FieldMap::new(),
            crate::CompiledQuery {
                tenant: self.tenant().map(str::to_owned),
                ..Default::default()
            },
        )?;
        let filter = filter
            .map(|filter| {
                crate::plan_filter(
                    resource,
                    self.authorization_actor(),
                    self.tenant(),
                    crate::TrustedExpr::Filter(filter.into_filter()),
                )
            })
            .transpose()?;
        for predicate in query.filter.iter().chain(filter.iter()) {
            predicate.validate_snapshot(resource)?;
        }
        Ok(EventReader {
            context: self.identity_context(),
            resource,
            predicate: filter,
            selection: None,
            metadata: Vec::new(),
        })
    }
}

/// Bind an ordinary notifier to a declared reader, rather than the default anonymous reader.
#[derive(Debug)]
pub struct ScopedNotifier {
    reader: EventReader,
    notifier: Arc<dyn Notifier>,
}
impl ScopedNotifier {
    pub fn new(reader: EventReader, notifier: Arc<dyn Notifier>) -> Self {
        Self { reader, notifier }
    }
}
impl Notifier for ScopedNotifier {
    fn notify_event<'a>(
        &'a self,
        event: &'a CommittedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(notification) = event.project(&self.reader)? {
                self.notifier.notify(&notification).await?;
            }
            Ok(())
        })
    }
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.notifier.notify(notification)
    }
}

/// Bootstrap-only disclosure of named record fields for one resource. This grant
/// bypasses read policies for its sink, including across tenants on the issuing Runtime.
/// Arguments are never disclosed. Actor IDs and metadata require separate declarations.
#[derive(Clone, Debug)]
pub struct NotifierGrant {
    authority: Arc<crate::context::RuntimeAuthority>,
    resource: &'static ResourceDef,
    fields: Vec<String>,
    metadata: Vec<MetadataDef>,
    actor_id: bool,
}
impl<D> Runtime<D> {
    pub fn notifier_grant<R: Resource>(&self, fields: &[&str]) -> Result<NotifierGrant> {
        self.notifier_grant_for(&R::DEF, fields)
    }
    pub fn notifier_grant_for(
        &self,
        resource: &'static ResourceDef,
        fields: &[&str],
    ) -> Result<NotifierGrant> {
        if fields
            .iter()
            .any(|field| resource.attribute(field).is_none())
        {
            return Err(Error::Invalid(
                "notifier grants require stored attribute names".into(),
            ));
        }
        // The bound context retains the issuer without exposing Runtime authority.
        let ctx = self.bootstrap();
        Ok(NotifierGrant {
            authority: Arc::clone(&ctx.scope.authority),
            resource,
            fields: fields.iter().map(|field| (*field).into()).collect(),
            metadata: Vec::new(),
            actor_id: false,
        })
    }
}
impl NotifierGrant {
    pub fn with_actor_id(mut self) -> Self {
        self.actor_id = true;
        self
    }
    pub fn with_metadata(mut self, definitions: &[MetadataDef]) -> Self {
        self.metadata = definitions.to_vec();
        self
    }
    pub fn attach(self, notifier: Arc<dyn Notifier>) -> Arc<dyn Notifier> {
        Arc::new(GrantedNotifier {
            grant: self,
            notifier,
        })
    }
}
#[derive(Debug)]
struct GrantedNotifier {
    grant: NotifierGrant,
    notifier: Arc<dyn Notifier>,
}
impl Notifier for GrantedNotifier {
    fn notify_event<'a>(
        &'a self,
        event: &'a CommittedEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !same_resource(event.resource, self.grant.resource)
                || !Arc::ptr_eq(&event.context.scope.authority, &self.grant.authority)
            {
                return Ok(());
            }
            let mut ctx = event.context.clone();
            ctx.scope = event.context.scope.system();
            let notification = event.project_admitted(
                &ctx,
                Some(&self.grant.fields),
                &self.grant.metadata,
                self.grant.actor_id,
            )?;
            self.notifier.notify(&notification).await
        })
    }
    fn notify<'a>(
        &'a self,
        notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.notifier.notify(notification)
    }
}

// Resource::DEF is an associated constant; separate promotions need not share an address.
fn same_resource(a: &ResourceDef, b: &ResourceDef) -> bool {
    a.name == b.name
        && a.table == b.table
        && a.store_name == b.store_name
        && (a.store_type_id)() == (b.store_type_id)()
}
