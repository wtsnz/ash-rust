use crate::action::{ActionDef, ActionKind, PersistKind};
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

/// Prepared write. Accept, changes, and validation have already run.
/// Persist happens on [`commit`](Self::commit).
pub struct Changeset<R: Resource> {
    action: &'static ActionDef,
    fields: FieldMap,
    arguments: FieldMap,
    existing: Option<R>,
    upsert: Option<(&'static str, Vec<String>)>,
}

impl<R: Resource> Changeset<R> {
    pub fn action(&self) -> &'static ActionDef {
        self.action
    }

    pub fn attributes(&self) -> &FieldMap {
        &self.fields
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
        Ok(Self {
            action,
            fields,
            arguments,
            existing: None,
            upsert: None,
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

        Ok(Self {
            action,
            fields,
            arguments,
            existing: Some(existing),
            upsert: None,
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

    pub async fn commit<D: DataLayer>(self, ctx: &Context<D>) -> Result<R> {
        expect_persist(self.action, PersistKind::DataLayer)?;
        authorize(&self, ctx)?;

        let id = required_uuid(&self.fields, pk_name(&R::DEF)?)?;
        let previous_fields = self.existing.as_ref().map(Resource::to_fields);
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
                        .upsert(&R::DEF, id, self.fields, identity, update_fields)
                        .await?
                } else {
                    ctx.data.create(&R::DEF, id, self.fields).await?
                }
            }
            ActionKind::Update => ctx.data.update(&R::DEF, id, self.fields).await?,
            kind => {
                return Err(Error::WrongActionKind {
                    action: self.action.name,
                    expected: "create or update",
                    actual: kind.as_str(),
                });
            }
        };
        crate::policy::redact_fields(&R::DEF, ctx.actor.as_ref(), &mut stored)?;
        let record = R::from_fields(&stored)?;

        let notification = crate::notifier::Notification::new(
            R::DEF.name,
            self.action.name,
            self.action.kind,
            id,
            stored,
            previous_fields,
            ctx.actor.clone(),
            self.arguments,
        );
        crate::notifier::dispatch_notification(ctx, &R::DEF, notification).await?;

        Ok(record)
    }

    pub fn into_record(self) -> Result<R> {
        R::from_fields(&self.fields)
    }
}

pub(crate) fn authorize<R: Resource, D>(changeset: &Changeset<R>, ctx: &Context<D>) -> Result<()> {
    let record = changeset
        .existing
        .as_ref()
        .map(Resource::to_fields)
        .unwrap_or_else(|| changeset.fields.clone());
    authorize_write(&R::DEF, changeset.action, ctx.actor.as_ref(), Some(&record))
}
