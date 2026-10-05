use crate::action::{ActionDef, ActionKind};
use crate::actor::Actor;
use crate::data_layer::Sort;
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

/// The filter `actor`'s read of `resource` through `action` must hold, or none.
///
/// Policies that settle against the actor alone, before any record is looked at, are decided
/// as Ash decides them: a read that no policy lets this actor make is `Forbidden`, not an
/// empty list. A check that reads the actor (`actor_present`, `relates_to(field)`,
/// `actor_eq(..)`) is false when there's no actor, so an anonymous read of a resource whose
/// policies all ask for one is `Forbidden`; and so is a read by an actor whose attributes
/// match no `authorize_if`. What depends on the record (`relates_to(field)` for an actor,
/// `is_nil(field)`) is left to the filter, which keeps the rows it holds for.
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
        return forbid_if_false(Filter::or(bypass_filters));
    }

    let mut parts = Vec::new();
    for policy in normal_policies {
        parts.push(policy_to_filter(policy, actor)?);
    }
    let normal_filter = Filter::and(parts);

    if bypass_filters.is_empty() {
        forbid_if_false(normal_filter)
    } else {
        let combined_bypass = Filter::or(bypass_filters);
        forbid_if_false(Filter::or([combined_bypass, normal_filter]))
    }
}

/// A filter that holds for no record, whatever the records are, is a read that policies
/// forbid: Ash's static `forbidden`.
fn forbid_if_false(filter: Filter) -> Result<Option<Filter>> {
    match filter {
        Filter::False => Err(Error::Forbidden),
        filter => Ok(Some(filter)),
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
    effects_to_filter(policy.checks, actor)
}

/// The records `effects` allow `actor`, as a filter: what [`eval_policy_effects`] decides
/// record by record.
pub fn effects_to_filter(effects: &[PolicyEffect], actor: Option<&Actor>) -> Result<Filter> {
    let mut forbids = Vec::new();
    let mut authorizes = Vec::new();

    for effect in effects {
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

impl Check {
    /// The record's fields this check reads.
    pub fn fields(&self, out: &mut Vec<&'static str>) {
        match self {
            Check::RelatesToActor { field } | Check::IsNil { field } | Check::Eq { field, .. } => out.push(field),
            Check::And(checks) | Check::Or(checks) => checks.iter().for_each(|check| check.fields(out)),
            Check::Always | Check::ActorPresent | Check::ActorAttributeEquals { .. } => {}
        }
    }
}

/// The record's fields `resource`'s field policies check: a read whose records are
/// redacted reads them, whatever it selects.
pub fn field_policy_fields(resource: &ResourceDef) -> Vec<&'static str> {
    let mut fields = Vec::new();
    for policy in resource.field_policies {
        for effect in policy.checks {
            let (PolicyEffect::AuthorizeIf(check)
            | PolicyEffect::AuthorizeUnless(check)
            | PolicyEffect::ForbidIf(check)
            | PolicyEffect::ForbidUnless(check)) = effect;
            check.fields(&mut fields);
        }
    }
    fields
}

pub fn redact_fields(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    fields: &mut FieldMap,
) -> Result<()> {
    // Every policy checks the record as it was read, then the fields they hide go: one
    // hidden first mustn't read as nil to a policy checking it after.
    for field in hidden_fields(resource, actor, fields)? {
        fields.insert(field.to_string(), crate::value::Value::Null);
    }
    Ok(())
}

/// The fields of `record` that `actor` may not see under `resource`'s field policies,
/// each policy checking the record as given.
pub fn hidden_fields(resource: &ResourceDef, actor: Option<&Actor>, record: &FieldMap) -> Result<Vec<&'static str>> {
    let mut hidden = Vec::new();
    for fp in resource.field_policies {
        if !eval_policy_effects(fp.checks, actor, Some(record))? {
            hidden.push(fp.field);
        }
    }
    Ok(hidden)
}

/// Where `actor` may read `field` of `resource` under its field policies, as a filter on
/// the record: `None` where nothing hides it (no field policy, or the primary key, which
/// Ash never hides).
fn field_condition(resource: &ResourceDef, field: &str, actor: Option<&Actor>) -> Result<Option<Filter>> {
    if resource.primary_key().is_some_and(|pk| pk.name == field) {
        return Ok(None);
    }
    let conditions = resource
        .field_policies
        .iter()
        .filter(|policy| policy.field == field)
        .map(|policy| effects_to_filter(policy.checks, actor))
        .collect::<Result<Vec<_>>>()?;
    Ok(match Filter::and(conditions) {
        Filter::True => None,
        condition => Some(condition),
    })
}

/// A client's filter on `resource`, as `actor` may run it: a field a field policy hides
/// reads as null where it's hidden, as Ash reads a client's reference to it (`if <policy>
/// then field else nil`). A filter can't find records by values the actor can't see.
///
/// Null makes a comparison unknown, which a filter excludes even negated, so a comparison
/// on a hidden field holds only where the field may be read, and its negation too.
/// Whether the field is nil holds wherever it's hidden.
pub fn guard_input_filter(resource: &ResourceDef, actor: Option<&Actor>, filter: Filter) -> Result<Filter> {
    guard_filter(resource, actor, filter, true)
}

fn guard_filter(resource: &ResourceDef, actor: Option<&Actor>, filter: Filter, positive: bool) -> Result<Filter> {
    let guard_all = |parts: Vec<Filter>| -> Result<Vec<Filter>> {
        parts.into_iter().map(|part| guard_filter(resource, actor, part, positive)).collect()
    };
    Ok(match filter {
        Filter::True | Filter::False => filter,
        Filter::And(parts) => Filter::and(guard_all(parts)?),
        Filter::Or(parts) => Filter::or(guard_all(parts)?),
        Filter::Not(inner) => !guard_filter(resource, actor, *inner, !positive)?,
        // The related rows a client's filter reaches are those the actor may read, as Ash
        // adds the destination read's authorization filter to each relationship path in
        // a client's filter: a filter can't find records by related rows hidden from it.
        Filter::Related { relationship, filter } => match resource.relationship(&relationship) {
            Some(rel) => {
                let dest = (rel.destination)();
                let inner = guard_filter(dest, actor, *filter, positive)?;
                let readable = match compile_read_filter(dest, dest.default_read(), actor) {
                    Ok(None) => inner,
                    Ok(Some(policy)) => Filter::and([inner, policy]),
                    Err(Error::Forbidden) => Filter::False,
                    Err(err) => return Err(err),
                };
                Filter::related(relationship, readable)
            }
            None => Filter::Related { relationship, filter },
        },
        leaf => {
            let mut fields = Vec::new();
            leaf.collect_fields(&mut fields);
            let condition = match fields.first() {
                Some(field) => field_condition(resource, field, actor)?,
                None => None,
            };
            match condition {
                None => leaf,
                Some(condition) if matches!(leaf, Filter::IsNil(_)) => Filter::or([!condition, leaf]),
                // Negated, `not (c and leaf)` would hold where it's hidden: `not c or leaf`
                // negates to `c and not leaf`.
                Some(condition) if positive => Filter::and([condition, leaf]),
                Some(condition) => Filter::or([!condition, leaf]),
            }
        }
    })
}

/// A client's sort on `resource`, as `actor` may run it: a field a field policy hides
/// sorts as null where it's hidden, as Ash sorts by a client's reference to it.
pub fn guard_input_sort(resource: &ResourceDef, actor: Option<&Actor>, sorts: Vec<Sort>) -> Result<Vec<Sort>> {
    sorts
        .into_iter()
        .map(|sort| {
            let guard = field_condition(resource, &sort.field, actor)?;
            Ok(Sort { guard, ..sort })
        })
        .collect()
}

/// The records `actor` may run the write `action` on, as a filter: what [`authorize_write`]
/// decides record by record, as Ash compiles a write's policies into an atomic update.
pub fn write_filter(resource: &ResourceDef, action: &ActionDef, actor: Option<&Actor>) -> Result<Filter> {
    if resource.policies.is_empty() {
        return Ok(Filter::True);
    }
    let applicable: Vec<_> = resource.policies.iter().filter(|policy| policy.applies(action)).collect();
    if applicable.is_empty() {
        return Ok(Filter::False);
    }
    let (bypass_policies, normal_policies): (Vec<_>, Vec<_>) =
        applicable.into_iter().partition(|p| p.bypass);
    let bypass: Vec<Filter> = bypass_policies
        .into_iter()
        .map(|policy| policy_to_filter(policy, actor))
        .collect::<Result<_>>()?;
    let normal = if normal_policies.is_empty() {
        Filter::False
    } else {
        Filter::and(
            normal_policies
                .into_iter()
                .map(|policy| policy_to_filter(policy, actor))
                .collect::<Result<Vec<_>>>()?,
        )
    };
    Ok(if bypass.is_empty() {
        normal
    } else {
        Filter::or(bypass.into_iter().chain([normal]))
    })
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
        indexes: &[],
        checks: &[],
        statements: &[],
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
        indexes: &[],
        checks: &[],
        statements: &[],
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
    fn missing_actor_is_forbidden_where_a_policy_relates_to_actor() {
        let err = compile_read_filter(&THING, &THING.actions[0], None).unwrap_err();
        assert!(matches!(err, Error::Forbidden));
    }

    #[test]
    fn an_actor_no_policy_lets_read_is_forbidden() {
        // QUEUE lets only a representative read; a customer settles to false before any row.
        let customer = Actor::new(Uuid::new_v4()).with("role", "customer");
        let err = compile_read_filter(&QUEUE, &QUEUE.actions[0], Some(&customer)).unwrap_err();
        assert!(matches!(err, Error::Forbidden));
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

        assert!(as_rep.matches(&unassigned));
        assert!(!as_rep.matches(&assigned));
        assert!(compile_read_filter(&QUEUE, &QUEUE.actions[0], Some(&customer)).is_err());
    }
}
