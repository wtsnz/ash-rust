use crate::action::{ActionDef, ActionKind};
use crate::actor::Actor;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::ResourceDef;
use crate::value::{ConstValue, FieldMap};

#[derive(Clone, Copy, Debug)]
pub struct PolicyDef {
    pub bypass: bool,
    pub when: PolicyWhen,
    pub checks: &'static [PolicyEffect],
}

impl PolicyDef {
    pub const fn when(when: PolicyWhen, checks: &'static [PolicyEffect]) -> Self {
        Self {
            bypass: false,
            when,
            checks,
        }
    }

    pub const fn bypass(when: PolicyWhen, checks: &'static [PolicyEffect]) -> Self {
        Self {
            bypass: true,
            when,
            checks,
        }
    }

    pub fn applies(&self, action: &ActionDef) -> bool {
        self.when.matches(action)
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PolicyWhen {
    Always,
    ActionType(ActionKind),
    ActionName(&'static str),
}

impl PolicyWhen {
    pub fn matches(&self, action: &ActionDef) -> bool {
        match self {
            Self::Always => true,
            Self::ActionType(kind) => action.kind == *kind,
            Self::ActionName(name) => action.name == *name,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FieldPolicyDef {
    pub field: &'static str,
    pub checks: &'static [PolicyEffect],
}

impl FieldPolicyDef {
    pub const fn new(field: &'static str, checks: &'static [PolicyEffect]) -> Self {
        Self { field, checks }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum PolicyEffect {
    AuthorizeIf(Check),
    AuthorizeUnless(Check),
    ForbidIf(Check),
    ForbidUnless(Check),
}

#[derive(Clone, Copy, Debug)]
pub enum Check {
    Always,
    ActorPresent,
    RelatesToActor {
        field: &'static str,
    },
    ActorAttributeEquals {
        attr: &'static str,
        value: ConstValue,
    },
    IsNil {
        field: &'static str,
    },
    Eq {
        field: &'static str,
        value: ConstValue,
    },
    And(&'static [Check]),
    Or(&'static [Check]),
}

pub fn compile_read_filter(
    resource: &ResourceDef,
    action: &ActionDef,
    actor: Option<&Actor>,
) -> Result<Option<Filter>> {
    if resource.policies.is_empty() {
        return Ok(None);
    }

    let applicable: Vec<_> = resource
        .policies
        .iter()
        .filter(|policy| policy.applies(action))
        .collect();
    if applicable.is_empty() {
        return Err(Error::Forbidden);
    }

    let (bypass_policies, normal_policies): (Vec<_>, Vec<_>) =
        applicable.into_iter().partition(|p| p.bypass);

    let mut bypass_filters = Vec::new();
    for bp in bypass_policies {
        let f = policy_to_filter(bp, actor)?;
        if f == Filter::True {
            return Ok(None);
        }
        bypass_filters.push(f);
    }

    if normal_policies.is_empty() {
        if bypass_filters.is_empty() {
            return Err(Error::Forbidden);
        }
        return Ok(Some(Filter::or(bypass_filters)));
    }

    let mut parts = Vec::new();
    for policy in normal_policies {
        parts.push(policy_to_filter(policy, actor)?);
    }
    let normal_filter = Filter::and(parts);

    if bypass_filters.is_empty() {
        Ok(Some(normal_filter))
    } else {
        let combined_bypass = Filter::or(bypass_filters);
        Ok(Some(Filter::or([combined_bypass, normal_filter])))
    }
}

pub fn authorize_write(
    resource: &ResourceDef,
    action: &ActionDef,
    actor: Option<&Actor>,
    record: Option<&FieldMap>,
) -> Result<()> {
    if resource.policies.is_empty() {
        return Ok(());
    }

    let applicable: Vec<_> = resource
        .policies
        .iter()
        .filter(|policy| policy.applies(action))
        .collect();
    if applicable.is_empty() {
        return Err(Error::Forbidden);
    }

    let (bypass_policies, normal_policies): (Vec<_>, Vec<_>) =
        applicable.into_iter().partition(|p| p.bypass);

    for bypass in bypass_policies {
        if eval_policy(bypass, actor, record)? {
            return Ok(());
        }
    }

    if normal_policies.is_empty() {
        return Err(Error::Forbidden);
    }

    for policy in normal_policies {
        if !eval_policy(policy, actor, record)? {
            return Err(Error::Forbidden);
        }
    }
    Ok(())
}

fn policy_to_filter(policy: &PolicyDef, actor: Option<&Actor>) -> Result<Filter> {
    let mut forbids = Vec::new();
    let mut authorizes = Vec::new();

    for effect in policy.checks {
        match effect {
            PolicyEffect::ForbidIf(check) => {
                let f = check_to_filter(check, actor)?;
                forbids.push(!f);
            }
            PolicyEffect::ForbidUnless(check) => {
                let f = check_to_filter(check, actor)?;
                forbids.push(f);
            }
            PolicyEffect::AuthorizeIf(check) => {
                let f = check_to_filter(check, actor)?;
                authorizes.push(f);
            }
            PolicyEffect::AuthorizeUnless(check) => {
                let f = check_to_filter(check, actor)?;
                authorizes.push(!f);
            }
        }
    }

    let auth_filter = if authorizes.is_empty() {
        Filter::True
    } else {
        Filter::or(authorizes)
    };

    if forbids.is_empty() {
        Ok(auth_filter)
    } else {
        let mut all = forbids;
        all.push(auth_filter);
        Ok(Filter::and(all))
    }
}

pub fn eval_policy_effects(
    effects: &[PolicyEffect],
    actor: Option<&Actor>,
    record: Option<&FieldMap>,
) -> Result<bool> {
    for effect in effects {
        match effect {
            PolicyEffect::ForbidIf(check) if eval_check(check, actor, record)? => {
                return Ok(false);
            }
            PolicyEffect::ForbidUnless(check) if !eval_check(check, actor, record)? => {
                return Ok(false);
            }
            _ => {}
        }
    }

    let mut has_authorizes = false;
    for effect in effects {
        match effect {
            PolicyEffect::AuthorizeIf(check) => {
                has_authorizes = true;
                if eval_check(check, actor, record)? {
                    return Ok(true);
                }
            }
            PolicyEffect::AuthorizeUnless(check) => {
                has_authorizes = true;
                if !eval_check(check, actor, record)? {
                    return Ok(true);
                }
            }
            _ => {}
        }
    }

    if !has_authorizes {
        return Ok(true);
    }

    Ok(false)
}

fn eval_policy(
    policy: &PolicyDef,
    actor: Option<&Actor>,
    record: Option<&FieldMap>,
) -> Result<bool> {
    eval_policy_effects(policy.checks, actor, record)
}

pub fn redact_fields(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    fields: &mut FieldMap,
) -> Result<()> {
    for fp in resource.field_policies {
        let is_allowed = eval_policy_effects(fp.checks, actor, Some(fields))?;
        if !is_allowed {
            fields.insert(fp.field.to_string(), crate::value::Value::Null);
        }
    }
    Ok(())
}

pub fn authorize_field_writes(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    record: Option<&FieldMap>,
    input_fields: &FieldMap,
) -> Result<()> {
    for fp in resource.field_policies {
        if input_fields.contains_key(fp.field) {
            let is_allowed = eval_policy_effects(fp.checks, actor, record)?;
            if !is_allowed {
                return Err(Error::Forbidden);
            }
        }
    }
    Ok(())
}

pub fn check_to_filter(check: &Check, actor: Option<&Actor>) -> Result<Filter> {
    match check {
        Check::Always => Ok(Filter::True),
        Check::ActorPresent => Ok(if actor.is_some() {
            Filter::True
        } else {
            Filter::False
        }),
        Check::RelatesToActor { field } => match actor {
            Some(actor) => Ok(Filter::eq(*field, actor.id)),
            None => Ok(Filter::False),
        },
        Check::ActorAttributeEquals { attr, value } => {
            Ok(if actor.is_some_and(|actor| actor.attr_eq(attr, *value)) {
                Filter::True
            } else {
                Filter::False
            })
        }
        Check::IsNil { field } => Ok(Filter::is_nil(*field)),
        Check::Eq { field, value } => Ok(Filter::eq(*field, *value)),
        Check::And(checks) => {
            let parts = checks
                .iter()
                .map(|check| check_to_filter(check, actor))
                .collect::<Result<Vec<_>>>()?;
            Ok(Filter::and(parts))
        }
        Check::Or(checks) => {
            let parts = checks
                .iter()
                .map(|check| check_to_filter(check, actor))
                .collect::<Result<Vec<_>>>()?;
            Ok(Filter::or(parts))
        }
    }
}

fn eval_check(check: &Check, actor: Option<&Actor>, record: Option<&FieldMap>) -> Result<bool> {
    let filter = check_to_filter(check, actor)?;
    match record {
        Some(record) => Ok(filter.matches(record)),
        None => Ok(matches!(filter, Filter::True)),
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::action::ActionDef;
    use crate::fields;
    use crate::resource::{AttrType, AttributeDef, DataLayerKind, ResourceDef};

    const THING: ResourceDef = ResourceDef {
        name: "Thing",
        table: "things",
        attributes: &[
            AttributeDef::uuid_pk("id"),
            AttributeDef::required("owner_id", AttrType::Uuid),
        ],
        relationships: &[],
        actions: &[ActionDef::read("read").primary()],
        policies: &[PolicyDef::when(
            PolicyWhen::ActionType(ActionKind::Read),
            &[PolicyEffect::AuthorizeIf(Check::RelatesToActor {
                field: "owner_id",
            })],
        )],
        field_policies: &[],
        calculations: &[],
        aggregates: &[],
        extensions: &[],
        notifiers: &[],
        identities: &[],
        embedded: false,
        data_layer: DataLayerKind::Memory,
        timestamps: None,
        store_type_id: crate::store::default_store_type_id,
        store_name: "DefaultStore",
        multitenancy: None,
    };

    const QUEUE: ResourceDef = ResourceDef {
        name: "Ticket",
        table: "tickets",
        attributes: &[],
        relationships: &[],
        actions: &[ActionDef::read("read").primary()],
        policies: &[PolicyDef::when(
            PolicyWhen::ActionType(ActionKind::Read),
            &[PolicyEffect::AuthorizeIf(Check::And(&[
                Check::IsNil {
                    field: "representative_id",
                },
                Check::ActorAttributeEquals {
                    attr: "role",
                    value: ConstValue::Str("representative"),
                },
            ]))],
        )],
        field_policies: &[],
        calculations: &[],
        aggregates: &[],
        extensions: &[],
        notifiers: &[],
        identities: &[],
        embedded: false,
        data_layer: DataLayerKind::Memory,
        timestamps: None,
        store_type_id: crate::store::default_store_type_id,
        store_name: "DefaultStore",
        multitenancy: None,
    };

    #[test]
    fn read_filter_keeps_rows_related_to_actor() {
        let owner = Uuid::new_v4();
        let other = Uuid::new_v4();
        let actor = Actor::new(owner);
        let filter = compile_read_filter(&THING, &THING.actions[0], Some(&actor))
            .unwrap()
            .unwrap();

        assert!(filter.matches(&fields! { "owner_id" => owner }));
        assert!(!filter.matches(&fields! { "owner_id" => other }));
    }

    #[test]
    fn missing_actor_matches_nothing_on_relates_to_actor() {
        let filter = compile_read_filter(&THING, &THING.actions[0], None)
            .unwrap()
            .unwrap();
        assert!(!filter.matches(&fields! { "owner_id" => Uuid::new_v4() }));
    }

    #[test]
    fn actor_attribute_and_nil_compile_to_record_filter() {
        let rep = Actor::new(Uuid::new_v4()).with("role", "representative");
        let customer = Actor::new(Uuid::new_v4()).with("role", "customer");
        let unassigned = fields! { "representative_id" => Option::<Uuid>::None };
        let assigned = fields! { "representative_id" => Uuid::new_v4() };

        let as_rep = compile_read_filter(&QUEUE, &QUEUE.actions[0], Some(&rep))
            .unwrap()
            .unwrap();
        let as_customer = compile_read_filter(&QUEUE, &QUEUE.actions[0], Some(&customer))
            .unwrap()
            .unwrap();

        assert!(as_rep.matches(&unassigned));
        assert!(!as_rep.matches(&assigned));
        assert!(!as_customer.matches(&unassigned));
    }
}
