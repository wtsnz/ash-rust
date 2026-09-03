use crate::action::{ActionDef, ActionKind, Change, ManagedRelType, PersistKind};
use crate::context::Context;
use crate::data_layer::DataLayer;
use crate::error::{Error, Result};
use crate::pipeline::{
    action_named, apply_changes, expect_kind, expect_persist, generate_pk, pk_name,
    run_validations, split_input, validate,
};
use crate::policy::authorize_write;
use crate::resource::Resource;
use crate::value::{FieldMap, Value, required_uuid};

/// Converts a type into a [`FieldMap`] for relationship mutations or nested inputs.
pub trait IntoFieldMap {
    fn into_field_map(self) -> FieldMap;
}

impl IntoFieldMap for FieldMap {
    fn into_field_map(self) -> FieldMap {
        self
    }
}

impl IntoFieldMap for &FieldMap {
    fn into_field_map(self) -> FieldMap {
        self.clone()
    }
}

impl IntoFieldMap for Value {
    fn into_field_map(self) -> FieldMap {
        match self {
            Value::Map(m) => m,
            _ => FieldMap::new(),
        }
    }
}

impl<K, V> IntoFieldMap for Vec<(K, V)>
where
    K: Into<String>,
    V: Into<Value>,
{
    fn into_field_map(self) -> FieldMap {
        self.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }
}

impl<K, V, const N: usize> IntoFieldMap for [(K, V); N]
where
    K: Into<String> + Clone,
    V: Into<Value> + Clone,
{
    fn into_field_map(self) -> FieldMap {
        self.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }
}

/// Specifications for mutating or synchronizing a child relationship within a changeset.
#[derive(Clone, Debug)]
pub struct ManagedRelationshipSpec {
    pub relationship: &'static str,
    pub rel_type: ManagedRelType,
    pub inputs: Vec<FieldMap>,
}

/// Hook running before persistence with mutable access to the changeset.
pub type BeforeActionHook<R> = Box<dyn FnOnce(&mut Changeset<R>) -> Result<()> + Send + 'static>;

/// Hook running immediately after persistence with mutable access to the newly saved record.
pub type AfterActionHook<R> = Box<dyn FnOnce(&mut R) -> Result<()> + Send + 'static>;

/// Hook running after transaction completion (commit or rollback), receiving the result.
pub type AfterTransactionHook<R> = Box<dyn FnOnce(std::result::Result<&R, &Error>) + Send + 'static>;

/// Prepared write. Accept, changes, and validation have already run.
/// Persist happens on [`commit`](Self::commit).
pub struct Changeset<R: Resource> {
    action: &'static ActionDef,
    fields: FieldMap,
    arguments: FieldMap,
    existing: Option<R>,
    upsert: Option<(&'static str, Vec<String>)>,
    before_actions: Vec<BeforeActionHook<R>>,
    after_actions: Vec<AfterActionHook<R>>,
    after_transactions: Vec<AfterTransactionHook<R>>,
    managed_relationships: Vec<ManagedRelationshipSpec>,
}

impl<R: Resource> Changeset<R> {
    pub fn action(&self) -> &'static ActionDef {
        self.action
    }

    pub fn attributes(&self) -> &FieldMap {
        &self.fields
    }

    pub fn attributes_mut(&mut self) -> &mut FieldMap {
        &mut self.fields
    }

    pub fn get_attribute(&self, name: &str) -> Option<&Value> {
        self.fields.get(name)
    }

    pub fn change_attribute(&mut self, key: impl Into<String>, val: impl Into<Value>) -> &mut Self {
        self.fields.insert(key.into(), val.into());
        self
    }

    /// Register a hook to run immediately before persistence.
    /// May inspect or mutate changeset attributes, or return an error to abort the write.
    pub fn before_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut Changeset<R>) -> Result<()> + Send + 'static,
    {
        self.before_actions.push(Box::new(hook));
        self
    }

    /// Register a hook to run immediately after persistence within the transaction.
    /// Receives mutable access to the newly saved record.
    pub fn after_action<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut R) -> Result<()> + Send + 'static,
    {
        self.after_actions.push(Box::new(hook));
        self
    }

    /// Register a hook to run after the transaction finishes (or immediately if not transactional).
    /// Receives the final result (`Ok(&record)` or `Err(&error)`).
    pub fn after_transaction<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(std::result::Result<&R, &Error>) + Send + 'static,
    {
        self.after_transactions.push(Box::new(hook));
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
        let field_maps: Vec<FieldMap> = inputs.into_iter().map(|f| f.into_field_map()).collect();
        self.managed_relationships.push(ManagedRelationshipSpec {
            relationship,
            rel_type,
            inputs: field_maps,
        });
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
        &self.managed_relationships
    }

    pub fn arguments(&self) -> &FieldMap {
        &self.arguments
    }

    pub fn argument(&self, name: &str) -> Option<&Value> {
        self.arguments.get(name)
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
        expect_kind(action, ActionKind::Create)?;
        let (mut fields, arguments) = split_input(action, input)?;
        generate_pk(&R::DEF, &mut fields);

        if let Some(v_attr) = R::DEF.optimistic_lock_attribute()
            && (!fields.contains_key(v_attr) || fields.get(v_attr) == Some(&Value::Null))
        {
            fields.insert(v_attr.to_string(), Value::Int(1));
        }

        for attr in R::DEF.attributes {
            if let Some(def_fn) = attr.default_fn
                && (!fields.contains_key(attr.name) || fields.get(attr.name) == Some(&Value::Null))
            {
                fields.insert(attr.name.to_string(), def_fn());
            }
        }

        if let Some((created_at, updated_at)) = R::DEF.timestamps {
            let now = crate::resource::utc_now_iso8601();
            if !fields.contains_key(created_at) || fields.get(created_at) == Some(&Value::Null) {
                fields.insert(created_at.to_string(), Value::String(now.clone()));
            }
            if !fields.contains_key(updated_at) || fields.get(updated_at) == Some(&Value::Null) {
                fields.insert(updated_at.to_string(), Value::String(now));
            }
        }

        apply_changes(&mut fields, action, ctx.actor.as_ref(), &arguments)?;
        validate(&R::DEF, &fields)?;
        run_validations(&R::DEF, action, None, &fields, &arguments)?;
        crate::policy::authorize_field_writes(&R::DEF, ctx.actor.as_ref(), None, &fields)?;
        let managed_relationships = extract_managed_relationships::<R>(action, &arguments);
        Ok(Self {
            action,
            fields,
            arguments,
            existing: None,
            upsert: None,
            before_actions: Vec::new(),
            after_actions: Vec::new(),
            after_transactions: Vec::new(),
            managed_relationships,
        })
    }

    pub fn with_upsert(mut self, identity: &'static str, update_fields: &[&str]) -> Self {
        self.upsert = Some((
            identity,
            update_fields.iter().map(|s| s.to_string()).collect(),
        ));
        self
    }

    pub fn for_update_on<D: DataLayer>(
        ctx: &Context<D>,
        action: &str,
        existing: R,
        input: FieldMap,
    ) -> Result<Self> {
        let action = action_named(&R::DEF, action)?;
        expect_kind(action, ActionKind::Update)?;
        let (accepted, arguments) = split_input(action, input)?;
        let mut fields = existing.to_fields();
        let existing_fields = fields.clone();
        fields.extend(accepted.clone());

        if let Some(v_attr) = R::DEF.optimistic_lock_attribute() {
            let current_v = existing_fields
                .get(v_attr)
                .and_then(|v| match v {
                    Value::Int(n) => Some(*n),
                    _ => None,
                })
                .unwrap_or(1);
            fields.insert(v_attr.to_string(), Value::Int(current_v + 1));
        }

        if let Some((_created_at, updated_at)) = R::DEF.timestamps {
            let now = crate::resource::utc_now_iso8601();
            fields.insert(updated_at.to_string(), Value::String(now));
        }

        apply_changes(&mut fields, action, ctx.actor.as_ref(), &arguments)?;
        validate(&R::DEF, &fields)?;
        run_validations(&R::DEF, action, Some(&existing_fields), &fields, &arguments)?;
        crate::policy::authorize_field_writes(
            &R::DEF,
            ctx.actor.as_ref(),
            Some(&existing_fields),
            &accepted,
        )?;
        for fp in R::DEF.field_policies {
            if !accepted.contains_key(fp.field) {
                let can_read = crate::policy::eval_policy_effects(
                    fp.checks,
                    ctx.actor.as_ref(),
                    Some(&existing_fields),
                )?;
                if !can_read {
                    fields.remove(fp.field);
                }
            }
        }

        let managed_relationships = extract_managed_relationships::<R>(action, &arguments);
        Ok(Self {
            action,
            fields,
            arguments,
            existing: Some(existing),
            upsert: None,
            before_actions: Vec::new(),
            after_actions: Vec::new(),
            after_transactions: Vec::new(),
            managed_relationships,
        })
    }

    pub fn apply_embedded(action: &str, input: FieldMap) -> Result<R> {
        let action = action_named(&R::DEF, action)?;
        expect_kind(action, ActionKind::Create)?;
        let (mut fields, arguments) = split_input(action, input)?;

        for attr in R::DEF.attributes {
            if let Some(def_fn) = attr.default_fn
                && (!fields.contains_key(attr.name) || fields.get(attr.name) == Some(&Value::Null))
            {
                fields.insert(attr.name.to_string(), def_fn());
            }
        }

        if let Some((created_at, updated_at)) = R::DEF.timestamps {
            let now = crate::resource::utc_now_iso8601();
            if !fields.contains_key(created_at) || fields.get(created_at) == Some(&Value::Null) {
                fields.insert(created_at.to_string(), Value::String(now.clone()));
            }
            if !fields.contains_key(updated_at) || fields.get(updated_at) == Some(&Value::Null) {
                fields.insert(updated_at.to_string(), Value::String(now));
            }
        }

        apply_changes(&mut fields, action, None, &arguments)?;
        validate(&R::DEF, &fields)?;
        run_validations(&R::DEF, action, None, &fields, &arguments)?;
        R::from_fields(&fields)
    }

    pub fn apply_embedded_update(action: &str, existing: R, input: FieldMap) -> Result<R> {
        let action = action_named(&R::DEF, action)?;
        expect_kind(action, ActionKind::Update)?;
        let (accepted, arguments) = split_input(action, input)?;
        let mut fields = existing.to_fields();
        let existing_fields = fields.clone();
        fields.extend(accepted);

        if let Some((_created_at, updated_at)) = R::DEF.timestamps {
            let now = crate::resource::utc_now_iso8601();
            fields.insert(updated_at.to_string(), Value::String(now));
        }

        apply_changes(&mut fields, action, None, &arguments)?;
        validate(&R::DEF, &fields)?;
        run_validations(&R::DEF, action, Some(&existing_fields), &fields, &arguments)?;
        R::from_fields(&fields)
    }

    pub async fn commit<D: DataLayer>(mut self, ctx: &Context<D>) -> Result<R> {
        let after_tx_hooks = std::mem::take(&mut self.after_transactions);
        let res = self.commit_inner(ctx).await;
        match &res {
            Ok(record) => {
                for hook in after_tx_hooks {
                    hook(Ok(record));
                }
            }
            Err(err) => {
                for hook in after_tx_hooks {
                    hook(Err(err));
                }
            }
        }
        res
    }

    async fn commit_inner<D: DataLayer>(&mut self, ctx: &Context<D>) -> Result<R> {
        for hook in std::mem::take(&mut self.before_actions) {
            hook(self)?;
        }

        expect_persist(self.action, PersistKind::DataLayer)?;
        authorize(self, ctx)?;

        // Pre-validate managed relationships before any persistence
        for managed in &self.managed_relationships {
            if let Some(rel) = R::DEF.relationship(managed.relationship) {
                let dest_def = (rel.destination)();
                let child_create_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Create && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                let child_update_action = dest_def
                    .actions
                    .iter()
                    .find(|a| a.kind == ActionKind::Update && a.primary)
                    .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Update));

                let child_pk = pk_name(dest_def)?;
                for child_fields in &managed.inputs {
                    let is_update = child_fields.contains_key(child_pk) && managed.rel_type == ManagedRelType::DirectControl;
                    if is_update {
                        if let Some(act) = child_update_action {
                            run_validations(dest_def, act, None, child_fields, &FieldMap::new())?;
                        }
                    } else if let Some(act) = child_create_action {
                        run_validations(dest_def, act, None, child_fields, &FieldMap::new())?;
                    }
                }
            }
        }

        // Handle belongs_to relationships before creating/updating parent
        for managed in &mut self.managed_relationships {
            if let Some(rel) = R::DEF.relationship(managed.relationship)
                && rel.kind == crate::resource::RelKind::BelongsTo
            {
                let dest_def = (rel.destination)();
                let dest_pk = pk_name(dest_def)?;
                if let Some(mut child_fields) = managed.inputs.pop() {
                    let child_id = if let Ok(cid) = required_uuid(&child_fields, dest_pk) {
                        cid
                    } else {
                        let create_act = dest_def
                            .actions
                            .iter()
                            .find(|a| a.kind == ActionKind::Create && a.primary)
                            .or_else(|| dest_def.actions.iter().find(|a| a.kind == ActionKind::Create));
                        if let Some(create_act) = create_act {
                            let stored = Box::pin(crate::engine::create_dynamic(ctx, dest_def, create_act, child_fields)).await?;
                            required_uuid(&stored, dest_pk)?
                        } else {
                            generate_pk(dest_def, &mut child_fields);
                            let cid = required_uuid(&child_fields, dest_pk)?;
                            ctx.data.create(dest_def, cid, child_fields).await?;
                            cid
                        }
                    };
                    self.fields.insert(rel.source_attribute.to_string(), Value::from(child_id));
                }
            }
        }

        let id = required_uuid(&self.fields, pk_name(&R::DEF)?)?;
        let previous_fields = self.existing.as_ref().map(Resource::to_fields);
        let fields = std::mem::take(&mut self.fields);
        let mut stored = match self.action.kind {
            ActionKind::Create => {
                if let Some((ident_name, ref update_fields)) = self.upsert {
                    let identity = R::DEF.identity(ident_name).ok_or_else(|| {
                        Error::Invalid(format!(
                            "unknown identity `{ident_name}` for upsert on {}",
                            R::DEF.name
                        ))
                    })?;
                    ctx.data
                        .upsert(&R::DEF, id, fields, identity, update_fields)
                        .await?
                } else {
                    ctx.data.create(&R::DEF, id, fields).await?
                }
            }
            ActionKind::Update => ctx.data.update(&R::DEF, id, fields).await?,
            kind => {
                return Err(Error::WrongActionKind {
                    action: self.action.name,
                    expected: "create or update",
                    actual: kind.as_str(),
                });
            }
        };

        let managed_list = std::mem::take(&mut self.managed_relationships);
        if let Err(err) = crate::engine::handle_managed_relationships(ctx, &R::DEF, id, managed_list).await {
            if self.action.kind == ActionKind::Create {
                let _ = ctx.data.destroy(&R::DEF, id).await;
            }
            return Err(err);
        }

        crate::policy::redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
        let mut record = R::from_fields(&stored)?;

        for hook in std::mem::take(&mut self.after_actions) {
            hook(&mut record)?;
        }

        let arguments = std::mem::take(&mut self.arguments);
        let notification = crate::notifier::Notification::new(
            R::DEF.name,
            self.action.name,
            self.action.kind,
            id,
            stored,
            previous_fields,
            ctx.actor.clone(),
            arguments,
        );
        crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;

        Ok(record)
    }

    pub fn into_record(self) -> Result<R> {
        R::from_fields(&self.fields)
    }
}

fn extract_managed_relationships<R: Resource>(
    action: &ActionDef,
    arguments: &FieldMap,
) -> Vec<ManagedRelationshipSpec> {
    let mut managed_relationships = Vec::new();
    for change in action.changes {
        if let Change::ManageRelationship { relationship, rel_type } = change
            && let Some(val) = arguments.get(*relationship)
        {
            let mut inputs = Vec::new();
            match val {
                Value::Array(items) => {
                    for it in items {
                        if let Value::Map(m) = it {
                            inputs.push(m.clone());
                        }
                    }
                }
                Value::Map(m) => {
                    inputs.push(m.clone());
                }
                _ => {}
            }
            managed_relationships.push(ManagedRelationshipSpec {
                relationship,
                rel_type: *rel_type,
                inputs,
            });
        }
    }
    for rel in R::DEF.relationships {
        if !managed_relationships.iter().any(|m| m.relationship == rel.name)
            && let Some(val) = arguments.get(rel.name)
        {
            let mut inputs = Vec::new();
            match val {
                Value::Array(items) => {
                    for it in items {
                        if let Value::Map(m) = it {
                            inputs.push(m.clone());
                        }
                    }
                }
                Value::Map(m) => {
                    inputs.push(m.clone());
                }
                _ => {}
            }
            if !inputs.is_empty() {
                managed_relationships.push(ManagedRelationshipSpec {
                    relationship: rel.name,
                    rel_type: crate::action::ManagedRelType::DirectControl,
                    inputs,
                });
            }
        }
    }
    managed_relationships
}

pub(crate) fn authorize<R: Resource, D>(changeset: &Changeset<R>, ctx: &Context<D>) -> Result<()> {
    let record = changeset
        .existing
        .as_ref()
        .map(Resource::to_fields)
        .unwrap_or_else(|| changeset.fields.clone());
    authorize_write(&R::DEF, changeset.action, ctx.actor.as_ref(), Some(&record))
}
