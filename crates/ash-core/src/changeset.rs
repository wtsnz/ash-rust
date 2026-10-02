mod dynamic;
mod input;
mod managed;

pub use dynamic::{DynamicChangeset, DynamicChangesetHook};
pub(crate) use dynamic::upsert_identity as dynamic_upsert_identity;
pub use input::IntoFieldMap;
pub use managed::ManagedRelationshipSpec;

use std::marker::PhantomData;

use crate::action::{ActionDef, ManagedRelType};
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::{Error, Result};
use crate::pipeline::action_named;
use crate::resource::Resource;
use crate::value::{FieldMap, Value};

/// Hook running before persistence with mutable access to the changeset.
pub type BeforeActionHook<R> = Box<dyn FnOnce(&mut Changeset<R>) -> Result<()> + Send + 'static>;

/// Hook running immediately after persistence with mutable access to the newly saved record.
pub type AfterActionHook<R> = Box<dyn FnOnce(&mut R) -> Result<()> + Send + 'static>;

/// Hook running after transaction completion (commit or rollback), receiving the result.
pub type AfterTransactionHook<R> =
    Box<dyn FnOnce(std::result::Result<&R, &Error>) + Send + 'static>;

/// Prepared write. Accept, changes, and validation have already run.
/// Persist happens on [`commit`](Self::commit).
///
/// A typed view of a [`DynamicChangeset`], which runs the write: typed hooks are
/// adapted to its fields, so typed and dynamic writes take the same steps.
pub struct Changeset<R: Resource> {
    inner: DynamicChangeset,
    existing: Option<R>,
    _resource: PhantomData<fn() -> R>,
}

impl<R: Resource> Changeset<R> {
    fn wrap(inner: DynamicChangeset, existing: Option<R>) -> Self {
        Self {
            inner,
            existing,
            _resource: PhantomData,
        }
    }

    /// The untyped changeset this one wraps.
    pub fn into_dynamic(self) -> DynamicChangeset {
        self.inner
    }

    pub fn action(&self) -> &'static ActionDef {
        self.inner.action()
    }

    pub fn tenant(&self) -> Option<&str> {
        self.inner.tenant.as_deref()
    }

    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.inner.tenant = Some(tenant.into());
        self
    }

    pub fn without_tenant(mut self) -> Self {
        self.inner.tenant = None;
        self
    }

    pub fn metadata(&self) -> &FieldMap {
        &self.inner.metadata
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.inner.metadata.insert(key.into(), value.into());
        self
    }

    pub fn attributes(&self) -> &FieldMap {
        self.inner.attributes()
    }

    pub fn attributes_mut(&mut self) -> &mut FieldMap {
        self.inner.attributes_mut()
    }

    pub fn get_attribute(&self, name: &str) -> Option<&Value> {
        self.inner.attributes().get(name)
    }

    pub fn change_attribute(&mut self, key: impl Into<String>, val: impl Into<Value>) -> &mut Self {
        self.inner.attributes_mut().insert(key.into(), val.into());
        self
    }

    /// Register a hook to run immediately before persistence.
    /// May inspect or mutate changeset attributes, or return an error to abort the write.
    pub fn before_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut Changeset<R>) -> Result<()> + Send + 'static,
    {
        self.inner = self.inner.before_action(move |inner: &mut DynamicChangeset| {
            let existing = inner.existing().map(R::from_fields).transpose()?;
            let mut typed = Changeset::wrap(inner.take(), existing);
            let result = hook(&mut typed);
            *inner = typed.inner;
            result
        });
        self
    }

    /// Register a hook to run immediately after persistence within the transaction.
    /// Receives mutable access to the newly saved record.
    pub fn after_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut R) -> Result<()> + Send + 'static,
    {
        self.inner = self.inner.after_action(move |stored: &mut FieldMap| {
            let mut record = R::from_fields(stored)?;
            hook(&mut record)?;
            *stored = record.to_fields();
            Ok(())
        });
        self
    }

    /// Register a hook to run after the transaction finishes (or immediately if not transactional).
    /// Receives the final result (`Ok(&record)` or `Err(&error)`).
    pub fn after_transaction<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(std::result::Result<&R, &Error>) + Send + 'static,
    {
        self.inner = self.inner.after_transaction(move |result| match result {
            Ok(stored) => match R::from_fields(stored) {
                Ok(record) => hook(Ok(&record)),
                Err(err) => hook(Err(&err)),
            },
            Err(err) => hook(Err(err)),
        });
        self
    }

    /// Register a nested child relationship mutation to execute alongside the changeset.
    pub fn manage_relationship<I, F>(
        mut self,
        relationship: &'static str,
        inputs: I,
        rel_type: ManagedRelType,
    ) -> Self
    where
        I: IntoIterator<Item = F>,
        F: IntoFieldMap,
    {
        let inputs = inputs.into_iter().map(|f| f.into_field_map()).collect();
        self.inner = self.inner.manage_relationship(relationship, inputs, rel_type);
        self
    }

    /// Register a single nested child relationship mutation.
    pub fn manage_relationship_one(
        self,
        relationship: &'static str,
        input: impl IntoFieldMap,
        rel_type: ManagedRelType,
    ) -> Self {
        self.manage_relationship(relationship, vec![input.into_field_map()], rel_type)
    }

    pub fn managed_relationships(&self) -> &[ManagedRelationshipSpec] {
        self.inner.managed_relationships()
    }

    pub fn arguments(&self) -> &FieldMap {
        self.inner.arguments()
    }

    pub fn argument(&self, name: &str) -> Option<&Value> {
        self.inner.arguments().get(name)
    }

    pub fn data(&self) -> Option<&R> {
        self.existing.as_ref()
    }

    pub fn for_create<D: DataLayer>(
        ctx: &Context<D>,
        action: &str,
        input: FieldMap,
    ) -> Result<Self> {
        let action = action_named(&R::DEF, action)?;
        let inner = DynamicChangeset::for_create(ctx, &R::DEF, action, input)?;
        Ok(Self::wrap(inner, None))
    }

    pub fn with_upsert(mut self, identity: &'static str, update_fields: &[&str]) -> Self {
        self.inner = self.inner.with_upsert(identity, update_fields);
        self
    }

    pub fn for_update_on<D: DataLayer>(
        ctx: &Context<D>,
        action: &str,
        existing: R,
        input: FieldMap,
    ) -> Result<Self> {
        let action = action_named(&R::DEF, action)?;
        let inner = DynamicChangeset::for_update(ctx, &R::DEF, action, existing.to_fields(), input)?;
        Ok(Self::wrap(inner, Some(existing)))
    }

    /// The record a create action would store, without a context or data layer. Like
    /// Ash's `apply_attributes`, it sets the primary key, defaults, and timestamps and runs
    /// the action's changes and validations, but checks no policies, sets no tenant, and
    /// manages no relationships. `Resource::build_<action>().build()` calls it.
    pub fn apply_create(action: &str, input: FieldMap) -> Result<R> {
        let action = action_named(&R::DEF, action)?;
        R::from_fields(&DynamicChangeset::apply_create(&R::DEF, action, input)?)
    }

    /// The record an update action would store, without a context or data layer. See
    /// [`Self::apply_create`] for what it leaves out.
    pub fn apply_update(action: &str, existing: R, input: FieldMap) -> Result<R> {
        let action = action_named(&R::DEF, action)?;
        let fields = DynamicChangeset::apply_update(&R::DEF, action, existing.to_fields(), input)?;
        R::from_fields(&fields)
    }

    pub fn for_destroy<D: DataLayer>(ctx: &Context<D>, action: &str, existing: R) -> Result<Self> {
        let action = action_named(&R::DEF, action)?;
        let inner = DynamicChangeset::for_destroy(ctx, &R::DEF, action, existing.to_fields())?;
        Ok(Self::wrap(inner, Some(existing)))
    }

    pub async fn commit<D: DataLayer>(self, ctx: &Context<D>) -> Result<R> {
        R::from_fields(&self.inner.commit(ctx).await?)
    }

    pub fn into_record(self) -> Result<R> {
        R::from_fields(self.inner.attributes())
    }
}

pub(crate) fn authorize<R: Resource, D>(changeset: &Changeset<R>, ctx: &Context<D>) -> Result<()> {
    changeset.inner.authorize(ctx)
}
