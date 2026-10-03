//! The one write pipeline. Typed changesets, the dynamic `*_dynamic` functions that
//! GraphQL calls, bulk actions and cascades all prepare, persist and finish their writes
//! here, so they run the same steps in the same order.

use uuid::Uuid;

use crate::action::{
    ActionDef, ActionKind, DynamicAfterActionHook, DynamicAfterTransactionHook, ManagedRelType,
    PersistKind,
};
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::engine::Cascade;
use crate::error::{Error, Result};
use crate::pipeline::{
    action_named, apply_changes_with_context, apply_tenant_to_fields, expect_kind,
    expect_persist, generate_pk, pk_name, prepare_create_fields, prepare_update_fields,
    run_validations, run_validations_with_context, split_input, validate,
};
use crate::policy::authorize_write;
use crate::resource::ResourceDef;
use crate::value::{FieldMap, Value, required_uuid};

use super::managed::{ManagedRelationshipSpec, extract_managed_relationships};
use crate::engine::atomic::{AtomicPlan, PlanInput, plan_update, run_atomic_destroy, run_atomic_update};

/// Hook running before persistence with mutable access to the changeset.
pub type DynamicChangesetHook =
    Box<dyn FnOnce(&mut DynamicChangeset) -> Result<()> + Send + 'static>;

/// A prepared write to any resource, in fields rather than a typed record. Building one
/// runs the action's accept list, changes and validations and checks field policies;
/// [`commit`](Self::commit) runs its before-action hooks, authorizes, persists, and then
/// runs after-action hooks and notifies.
pub struct DynamicChangeset {
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    fields: FieldMap,
    arguments: FieldMap,
    pub tenant: Option<String>,
    pub metadata: FieldMap,
    existing: Option<FieldMap>,
    upsert: Option<(&'static str, Vec<String>)>,
    before_actions: Vec<DynamicChangesetHook>,
    after_actions: Vec<DynamicAfterActionHook>,
    after_transactions: Vec<DynamicAfterTransactionHook>,
    managed_relationships: Vec<ManagedRelationshipSpec>,
}

impl DynamicChangeset {
    fn new(
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        fields: FieldMap,
        arguments: FieldMap,
        existing: Option<FieldMap>,
    ) -> Self {
        Self {
            resource,
            action,
            fields,
            arguments,
            tenant: None,
            metadata: FieldMap::new(),
            existing,
            upsert: None,
            before_actions: Vec::new(),
            after_actions: Vec::new(),
            after_transactions: Vec::new(),
            managed_relationships: Vec::new(),
        }
    }

    /// Runs the action's changes, keeping the hooks they register.
    fn apply_changes<D>(&mut self, ctx: &Context<D>) -> Result<()> {
        let mut before_actions = Vec::new();
        apply_changes_with_context(
            &mut self.fields,
            self.action,
            ctx.actor.as_ref(),
            ctx.tenant(),
            ctx.metadata(),
            &self.arguments,
            &mut before_actions,
            &mut self.after_actions,
            &mut self.after_transactions,
        )?;
        for hook in before_actions {
            self.before_actions
                .push(Box::new(move |cs: &mut DynamicChangeset| hook(&mut cs.fields)));
        }
        Ok(())
    }

    fn run_validations<D>(&self, ctx: &Context<D>) -> Result<()> {
        run_validations_with_context(
            self.resource,
            self.action,
            self.existing.as_ref(),
            &self.fields,
            ctx.actor.as_ref(),
            ctx.tenant(),
            ctx.metadata(),
            &self.arguments,
        )
    }

    fn with_context<D>(mut self, ctx: &Context<D>) -> Self {
        self.tenant = ctx.tenant.clone();
        self.metadata = ctx.metadata.clone();
        self.managed_relationships = extract_managed_relationships(self.resource, self.action, &self.arguments);
        self
    }

    /// A create through `action`, with `input` split into accepted attributes and
    /// arguments.
    pub fn for_create<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        input: FieldMap,
    ) -> Result<Self> {
        Self::for_create_forcing(ctx, resource, action, input, FieldMap::new())
    }

    /// [`for_create`](Self::for_create) with `forced` attributes set whether or not the
    /// action accepts them, as Ash's `force_change_attribute` does. Managed
    /// relationships force the keys that link a record to its parent.
    pub(crate) fn for_create_forcing<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        input: FieldMap,
        forced: FieldMap,
    ) -> Result<Self> {
        expect_kind(action, ActionKind::Create)?;
        let (mut fields, arguments) = split_input(action, input)?;
        fields.extend(forced);
        prepare_create_fields(resource, &mut fields);
        let mut changeset = Self::new(resource, action, fields, arguments, None);
        changeset.apply_changes(ctx)?;
        apply_tenant_to_fields(resource, &mut changeset.fields, ctx.tenant(), true)?;
        changeset.run_validations(ctx)?;
        validate(resource, &mut changeset.fields)?;
        Ok(changeset.with_context(ctx))
    }

    /// An update of `existing` through `action`.
    pub fn for_update<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        existing: FieldMap,
        input: FieldMap,
    ) -> Result<Self> {
        Self::for_update_forcing(ctx, resource, action, existing, input, FieldMap::new())
    }

    /// [`for_update`](Self::for_update) with `forced` attributes set whether or not the
    /// action accepts them. See [`for_create_forcing`](Self::for_create_forcing).
    pub(crate) fn for_update_forcing<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        existing: FieldMap,
        input: FieldMap,
        forced: FieldMap,
    ) -> Result<Self> {
        expect_kind(action, ActionKind::Update)?;
        let (accepted, arguments) = split_input(action, input)?;
        let mut fields = existing.clone();
        fields.extend(accepted.clone());
        fields.extend(forced);
        prepare_update_fields(resource, &existing, &mut fields);
        let mut changeset = Self::new(resource, action, fields, arguments, Some(existing));
        changeset.apply_changes(ctx)?;
        apply_tenant_to_fields(resource, &mut changeset.fields, ctx.tenant(), false)?;
        changeset.run_validations(ctx)?;
        validate(resource, &mut changeset.fields)?;
        // Field policies govern what's read, not what's written, as in Ash: writes are the
        // action's policies' to authorize. A field the actor may not read is not written
        // back, though: a typed record read by that actor holds a redacted null there, not
        // the stored value.
        let existing = changeset.existing.as_ref();
        for fp in resource.field_policies {
            if !accepted.contains_key(fp.field)
                && !crate::policy::eval_policy_effects(fp.checks, ctx.actor.as_ref(), existing)?
            {
                changeset.fields.remove(fp.field);
            }
        }
        Ok(changeset.with_context(ctx))
    }

    /// A destroy of `existing` through `action`.
    pub fn for_destroy<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        existing: FieldMap,
    ) -> Result<Self> {
        Self::for_destroy_with(ctx, resource, action, existing, FieldMap::new())
    }

    /// [`for_destroy`](Self::for_destroy) with `input`: the action's arguments, and the
    /// attributes it accepts, which a soft destroy writes.
    pub fn for_destroy_with<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        existing: FieldMap,
        input: FieldMap,
    ) -> Result<Self> {
        expect_kind(action, ActionKind::Destroy)?;
        let (accepted, arguments) = crate::pipeline::split_input(action, input)?;
        let mut fields = existing.clone();
        fields.extend(accepted);
        let mut changeset = Self::new(resource, action, fields, arguments, Some(existing));
        changeset.apply_changes(ctx)?;
        changeset.run_validations(ctx)?;
        Ok(changeset.with_context(ctx))
    }

    /// [`for_create`](Self::for_create) through the action named `action`.
    pub fn for_create_named<D>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &str,
        input: FieldMap,
    ) -> Result<Self> {
        Self::for_create(ctx, resource, action_named(resource, action)?, input)
    }

    /// The record a create would store, without a context or data layer: the primary
    /// key, defaults and timestamps, and the action's changes and validations, but no
    /// policies, tenant or managed relationships.
    pub fn apply_create(
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        input: FieldMap,
    ) -> Result<FieldMap> {
        expect_kind(action, ActionKind::Create)?;
        let (mut fields, arguments) = split_input(action, input)?;
        prepare_create_fields(resource, &mut fields);
        crate::pipeline::apply_changes(&mut fields, action, None, &arguments)?;
        validate(resource, &mut fields)?;
        run_validations(resource, action, None, &fields, &arguments)?;
        Ok(fields)
    }

    /// The record an update would store, without a context or data layer. See
    /// [`apply_create`](Self::apply_create) for what it leaves out.
    pub fn apply_update(
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        existing: FieldMap,
        input: FieldMap,
    ) -> Result<FieldMap> {
        expect_kind(action, ActionKind::Update)?;
        let (accepted, arguments) = split_input(action, input)?;
        let mut fields = existing.clone();
        fields.extend(accepted);
        prepare_update_fields(resource, &existing, &mut fields);
        crate::pipeline::apply_changes(&mut fields, action, None, &arguments)?;
        validate(resource, &mut fields)?;
        run_validations(resource, action, Some(&existing), &fields, &arguments)?;
        Ok(fields)
    }

    pub fn resource(&self) -> &'static ResourceDef {
        self.resource
    }

    pub fn action(&self) -> &'static ActionDef {
        self.action
    }

    pub fn attributes(&self) -> &FieldMap {
        &self.fields
    }

    pub fn attributes_mut(&mut self) -> &mut FieldMap {
        &mut self.fields
    }

    pub fn arguments(&self) -> &FieldMap {
        &self.arguments
    }

    /// The record being updated or destroyed, as it was.
    pub fn existing(&self) -> Option<&FieldMap> {
        self.existing.as_ref()
    }

    pub fn managed_relationships(&self) -> &[ManagedRelationshipSpec] {
        &self.managed_relationships
    }

    pub fn with_upsert(mut self, identity: &'static str, update_fields: &[&str]) -> Self {
        self.upsert = Some((
            identity,
            update_fields.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    /// Runs `hook` immediately before persistence. It may change the attributes, or
    /// return an error to abort the write.
    pub fn before_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut DynamicChangeset) -> Result<()> + Send + 'static,
    {
        self.before_actions.push(Box::new(hook));
        self
    }

    /// Runs `hook` on the stored record immediately after persistence.
    pub fn after_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut FieldMap) -> Result<()> + Send + 'static,
    {
        self.after_actions.push(Box::new(hook));
        self
    }

    /// Runs `hook` with the write's outcome once it has finished.
    pub fn after_transaction<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(std::result::Result<&FieldMap, &Error>) + Send + 'static,
    {
        self.after_transactions.push(Box::new(hook));
        self
    }

    /// Manages a relationship's records alongside this write.
    pub fn manage_relationship(
        mut self,
        relationship: &'static str,
        inputs: Vec<FieldMap>,
        rel_type: ManagedRelType,
    ) -> Self {
        self.managed_relationships.push(ManagedRelationshipSpec {
            relationship,
            rel_type,
            inputs,
        });
        self
    }

    /// An empty changeset in this one's place, so a hook can take it by value.
    pub(crate) fn take(&mut self) -> Self {
        let empty = Self::new(self.resource, self.action, FieldMap::new(), FieldMap::new(), None);
        std::mem::replace(self, empty)
    }

    /// Checks the actor may run the action on this record: the record as it was for an
    /// update or destroy, as in Ash, or the new record for a create.
    pub(crate) fn authorize<D>(&self, ctx: &Context<D>) -> Result<()> {
        let record = self.existing.as_ref().unwrap_or(&self.fields);
        authorize_write(self.resource, self.action, ctx.actor.as_ref(), Some(record))
    }

    /// Persists the write and finishes it.
    pub async fn commit<D: DataLayer>(self, ctx: &Context<D>) -> Result<FieldMap> {
        self.commit_within(ctx, &Cascade::new(true)).await
    }

    /// [`commit`](Self::commit) inside a destroy `cascade`, which decides whether it
    /// notifies.
    pub(crate) async fn commit_within<D: DataLayer>(
        mut self,
        ctx: &Context<D>,
        cascade: &Cascade,
    ) -> Result<FieldMap> {
        let after_transactions = std::mem::take(&mut self.after_transactions);
        let result = async {
            if let Some(plan) = self.atomic_plan(ctx)? {
                return self.persist_atomically(ctx, plan, cascade).await;
            }
            let id = self.prepare(ctx).await?;
            let stored = self.persist(ctx, id, cascade).await?;
            self.finish(ctx, id, stored, cascade.notify).await
        }
        .await;
        for hook in after_transactions {
            hook(result.as_ref());
        }
        result
    }

    /// The update of the record in hand as one statement, as Ash upgrades an update of a
    /// record to an atomic one: what this changeset changes, with the action's changes as
    /// expressions and its validations, policies and lock version as conditions. A soft
    /// destroy is an update here, as in Ash; a hard destroy of a record in hand isn't
    /// upgraded, as in Ash. `None` when it runs record by record: it isn't an update, its
    /// data layer can't, or it can't and doesn't have to (`require_atomic`, else an error).
    fn atomic_plan<D: DataLayer>(&self, ctx: &Context<D>) -> Result<Option<AtomicPlan>> {
        let Some(existing) = self.existing.as_ref() else {
            return Ok(None);
        };
        let updates = match self.action.kind {
            ActionKind::Update => true,
            ActionKind::Destroy => self.action.soft,
            _ => false,
        };
        if !updates || !ctx.data.can_update_atomically(self.resource) {
            return Ok(None);
        }
        let planned = if !self.before_actions.is_empty() {
            Err("it has a before_action hook".to_string())
        } else if !self.managed_relationships.is_empty() {
            Err("it manages relationships".to_string())
        } else {
            let id = required_uuid(existing, pk_name(self.resource)?)?;
            let expected_version = self
                .resource
                .optimistic_lock_attribute()
                .map(|version| (id, existing.get(version).and_then(Value::as_int).unwrap_or(1)));
            plan_update(
                self.resource,
                self.action,
                PlanInput {
                    actor: ctx.actor.as_ref(),
                    tenant: ctx.tenant(),
                    sets: self.changes(self.fields.clone()),
                    arguments: &self.arguments,
                    expected_version,
                    collect_hooks: false,
                },
            )?
        };
        match planned {
            Ok(plan) => Ok(Some(plan)),
            Err(reason) if self.action.require_atomic => Err(Error::MustBeAtomic {
                resource: self.resource.name,
                action: self.action.name,
                reason,
            }),
            Err(_) => Ok(None),
        }
    }

    /// Runs `plan` against the record in hand. No row means it changed (or went) since it
    /// was read: [`Error::StaleRecord`], as in Ash. A soft destroy archives its children
    /// after, as one run record by record does.
    async fn persist_atomically<D: DataLayer>(&mut self, ctx: &Context<D>, plan: AtomicPlan, cascade: &Cascade) -> Result<FieldMap> {
        let existing = self.existing.as_ref().ok_or(Error::NotFound)?;
        let id = required_uuid(existing, pk_name(self.resource)?)?;
        let destroy = self.action.kind == ActionKind::Destroy;
        let stored = if destroy && !cascade.enter(self.resource, id) {
            // Already being destroyed further up a cascade.
            existing.clone()
        } else {
            let stored = run_atomic_update(ctx, self.resource, self.action, id, &plan.update, None)
                .await?
                .ok_or(Error::StaleRecord { resource: self.resource.name, id })?;
            if destroy {
                crate::engine::cascade_destroy_related(ctx, self.resource, self.action, id, &stored, cascade).await?;
            }
            stored
        };
        self.after_actions.extend(plan.after_actions);
        self.finish(ctx, id, stored, cascade.notify).await
    }

    /// An update or destroy of record `id`, by id, as one statement: no read first. A
    /// hard destroy deletes it, returning what it held; a soft destroy updates it and
    /// archives its children after, within `scope` when given. No row means no such
    /// record the context may see: [`Error::NotFound`].
    pub(crate) async fn commit_atomic_by_id<D: DataLayer>(
        ctx: &Context<D>,
        resource: &'static ResourceDef,
        action: &'static ActionDef,
        id: Uuid,
        arguments: FieldMap,
        plan: AtomicPlan,
        scope: Option<crate::filter::Filter>,
    ) -> Result<FieldMap> {
        let mut changeset = Self::new(resource, action, FieldMap::new(), arguments, None);
        changeset.after_actions = plan.after_actions;
        let result = async {
            let stored = match action.kind {
                ActionKind::Destroy if !action.soft => {
                    let destroyed = run_atomic_destroy(ctx, resource, action, id, &plan.update.conditions, scope.as_ref())
                        .await?
                        .ok_or(Error::NotFound)?;
                    // The record a destroy's notification carries, as one read first does.
                    changeset.existing = Some(destroyed.clone());
                    destroyed
                }
                ActionKind::Destroy => {
                    let cascade = Cascade::new(true);
                    cascade.enter(resource, id);
                    let stored = run_atomic_update(ctx, resource, action, id, &plan.update, scope.as_ref()).await?.ok_or(Error::NotFound)?;
                    crate::engine::cascade_destroy_related(ctx, resource, action, id, &stored, &cascade).await?;
                    stored
                }
                _ => run_atomic_update(ctx, resource, action, id, &plan.update, scope.as_ref()).await?.ok_or(Error::NotFound)?,
            };
            changeset.finish(ctx, id, stored, true).await
        }
        .await;
        for hook in plan.after_transactions {
            hook(result.as_ref());
        }
        result
    }

    /// Takes the after-transaction hooks, for a caller that runs them itself.
    pub(crate) fn take_after_transactions(&mut self) -> Vec<DynamicAfterTransactionHook> {
        std::mem::take(&mut self.after_transactions)
    }

    /// Everything before persistence: before-action hooks, validations again, the
    /// write policy, managed `belongs_to` records, and the tenant. Returns the record's
    /// primary key.
    pub(crate) async fn prepare<D: DataLayer>(&mut self, ctx: &Context<D>) -> Result<Uuid> {
        for hook in std::mem::take(&mut self.before_actions) {
            hook(self)?;
        }
        self.run_validations(ctx)?;
        if self.action.kind != ActionKind::Destroy {
            validate(self.resource, &mut self.fields)?;
        }
        expect_persist(self.action, PersistKind::DataLayer)?;
        self.authorize(ctx)?;
        self.validate_managed_inputs()?;
        self.attach_belongs_to(ctx).await?;
        apply_tenant_to_fields(
            self.resource,
            &mut self.fields,
            ctx.tenant(),
            self.action.kind == ActionKind::Create,
        )?;
        required_uuid(&self.fields, pk_name(self.resource)?)
    }

    /// Checks each managed record against its own create or update action before
    /// anything is written.
    fn validate_managed_inputs(&self) -> Result<()> {
        for managed in &self.managed_relationships {
            let Some(rel) = self.resource.relationship(managed.relationship) else {
                continue;
            };
            let dest_def = (rel.destination)();
            let child_pk = pk_name(dest_def)?;
            for child_fields in &managed.inputs {
                let is_update = child_fields.contains_key(child_pk)
                    && managed.rel_type == ManagedRelType::DirectControl;
                let kind = if is_update { ActionKind::Update } else { ActionKind::Create };
                if let Some(act) = primary_action(dest_def, kind) {
                    run_validations(dest_def, act, None, child_fields, &FieldMap::new())?;
                }
            }
        }
        Ok(())
    }

    /// Creates or finds each managed `belongs_to` record and copies its key onto this
    /// one, which must reference it before it is written.
    async fn attach_belongs_to<D: DataLayer>(&mut self, ctx: &Context<D>) -> Result<()> {
        for managed in &mut self.managed_relationships {
            let Some(rel) = self.resource.relationship(managed.relationship) else {
                continue;
            };
            if rel.kind != crate::resource::RelKind::BelongsTo {
                continue;
            }
            let dest_def = (rel.destination)();
            let dest_pk = pk_name(dest_def)?;
            let Some(mut child_fields) = managed.inputs.pop() else {
                continue;
            };
            // The related row, so every key column can be copied from it.
            let related: FieldMap = if let Ok(cid) = required_uuid(&child_fields, dest_pk) {
                if rel.destination_columns() == [dest_pk] {
                    child_fields
                } else {
                    ctx.data
                        .run_query(
                            dest_def,
                            &crate::data_layer::CompiledQuery {
                                filter: Some(crate::filter::Filter::eq(dest_pk, cid)),
                                tenant: ctx.tenant.clone(),
                                ..crate::data_layer::CompiledQuery::default()
                            },
                        )
                        .await?
                        .into_iter()
                        .next()
                        .ok_or(Error::NotFound)?
                }
            } else if let Some(create_act) = primary_action(dest_def, ActionKind::Create) {
                Box::pin(Self::for_create(ctx, dest_def, create_act, child_fields)?.commit(ctx))
                    .await?
            } else {
                generate_pk(dest_def, &mut child_fields);
                let cid = required_uuid(&child_fields, dest_pk)?;
                ctx.data.create(dest_def, ctx.tenant.as_deref(), cid, child_fields).await?
            };
            for (source, destination) in rel.key_pairs() {
                let value = related.get(destination).cloned().unwrap_or(Value::Null);
                self.fields.insert(source.to_string(), value);
            }
        }
        Ok(())
    }

    /// The prepared attributes, for a caller that writes them itself.
    pub(crate) fn take_fields(&mut self) -> FieldMap {
        std::mem::take(&mut self.fields)
    }

    /// What an update writes: the prepared attributes that differ from the record it
    /// started from. As in Ash, an update writes only the attributes it changes, merged
    /// into the stored row, so one made from a stale copy of the record doesn't write that
    /// copy's other fields back over newer values. Setting a field to the value the copy
    /// holds isn't a change either (`Ash.Changeset` drops it).
    pub(crate) fn changes(&self, fields: FieldMap) -> FieldMap {
        match &self.existing {
            Some(existing) => fields
                .into_iter()
                .filter(|(name, value)| existing.get(name) != Some(value))
                .collect(),
            None => fields,
        }
    }

    /// Writes the prepared record through the data layer.
    pub(crate) async fn persist<D: DataLayer>(
        &mut self,
        ctx: &Context<D>,
        id: Uuid,
        cascade: &Cascade,
    ) -> Result<FieldMap> {
        let fields = std::mem::take(&mut self.fields);
        match self.action.kind {
            ActionKind::Create => match &self.upsert {
                Some((identity_name, update_fields)) => {
                    let identity = upsert_identity(self.resource, identity_name)?;
                    ctx.data
                        .upsert(self.resource, ctx.tenant.as_deref(), id, fields, identity, update_fields)
                        .await
                }
                None => ctx.data.create(self.resource, ctx.tenant.as_deref(), id, fields).await,
            },
            ActionKind::Update => {
                let changes = self.changes(fields);
                ctx.data.update(self.resource, ctx.tenant.as_deref(), id, changes).await
            }
            ActionKind::Destroy => {
                let existing = self.existing.clone().unwrap_or_else(|| fields.clone());
                crate::engine::persist_destroy(
                    ctx,
                    self.resource,
                    self.action,
                    id,
                    &existing,
                    fields,
                    cascade,
                )
                .await
            }
            kind => Err(Error::WrongActionKind {
                action: self.action.name,
                expected: "create, update, or destroy",
                actual: kind.as_str(),
            }),
        }
    }

    /// Everything after persistence: managed relationships (undoing a create they
    /// fail), redaction, after-action hooks, and the notification when `notify`.
    pub(crate) async fn finish<D: DataLayer>(
        &mut self,
        ctx: &Context<D>,
        id: Uuid,
        mut stored: FieldMap,
        notify: bool,
    ) -> Result<FieldMap> {
        let managed = std::mem::take(&mut self.managed_relationships);
        if let Err(err) =
            crate::engine::handle_managed_relationships(ctx, self.resource, id, &stored, managed)
                .await
        {
            if self.action.kind == ActionKind::Create {
                let _ = ctx.data.destroy(self.resource, ctx.tenant.as_deref(), id).await;
            }
            return Err(err);
        }

        crate::policy::redact_fields(self.resource, ctx.actor.as_ref(), &mut stored)?;
        for hook in std::mem::take(&mut self.after_actions) {
            hook(&mut stored)?;
        }

        if notify {
            let mut metadata = ctx.metadata.clone();
            metadata.extend(self.metadata.clone());
            metadata.extend(self.arguments.clone());
            let notification = crate::notifier::Notification::new(
                self.resource.name,
                self.action.name,
                self.action.kind,
                id,
                stored.clone(),
                self.existing.clone(),
                ctx.actor.clone(),
                metadata,
            )
            .with_tenant(self.tenant.clone().or_else(|| ctx.tenant.clone()));
            crate::notifier::dispatch_notification(ctx, self.resource, notification).await?;
        }
        Ok(stored)
    }
}

/// The action of `kind` a write reaches for on its own: the primary one, or the first.
pub(crate) fn primary_action(
    resource: &'static ResourceDef,
    kind: ActionKind,
) -> Option<&'static ActionDef> {
    let of_kind = || resource.actions.iter().filter(move |a| a.kind == kind);
    of_kind().find(|a| a.primary).or_else(|| of_kind().next())
}

pub(crate) fn upsert_identity(
    resource: &'static ResourceDef,
    name: &str,
) -> Result<&'static crate::resource::IdentityDef> {
    resource.identity(name).ok_or_else(|| {
        Error::Invalid(format!(
            "unknown identity `{name}` for upsert on {}",
            resource.name
        ))
    })
}
