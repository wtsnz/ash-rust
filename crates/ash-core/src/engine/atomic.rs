//! Planning an update or destroy as one statement, as Ash's `fully_atomic_changeset`
//! does: the values an update sets as expressions over the stored record, and the
//! conditions (the action's validations, the write policies, the lock version) under
//! which it fails, each with its error. It can't be planned when something in it needs
//! the record in memory: a `before_action` hook, a change or validation function,
//! managed relationships, or a hard destroy's cascades.


use crate::action::{
    ActionDef, ActionKind, Change, DynamicAfterActionHook, DynamicAfterTransactionHook, PersistKind, Validation,
};
use crate::actor::Actor;
use crate::atomic::{Atomic, AtomicCondition, AtomicContext, AtomicExpr, AtomicUpdate};
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::pipeline::{check_builtin_validation, pk_name, validate_given};
use crate::policy::write_filter;
use crate::resource::{AttrType, OnDelete, RelKind, ResourceDef};
use crate::value::{FieldMap, Value};

/// An update planned as one statement, with the hooks that run after it.
pub(crate) struct AtomicPlan {
    pub update: AtomicUpdate,
    /// What a data layer that can't raise in its statement checks as filters instead.
    pub guards: Guards,
    pub after_actions: Vec<DynamicAfterActionHook>,
    pub after_transactions: Vec<DynamicAfterTransactionHook>,
}

/// Runs `update` on record `id` as `ctx` sees it, as one statement: through the read
/// `action` upgrades with (the primary read by default), in the context's tenant, and
/// within `scope` when given. `None` when there's no such record.
/// What an atomic update checks as filters where the data layer can't raise in its
/// statement, as Ash filters an update by its lock version (`optimistic_lock`) and
/// authorizes one by filter (`authorize_with: :filter`): an update that changes nothing
/// failed one of them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Guards {
    /// The lock version attribute and the version the record must still have.
    pub version: Option<(&'static str, i64)>,
    /// The records the write policies let the actor change.
    pub policy: Option<Filter>,
}

impl Guards {
    pub(crate) fn filter(&self) -> Option<Filter> {
        let version = self.version.map(|(field, expected)| Filter::eq(field, Value::Int(expected)));
        match (version, self.policy.clone()) {
            (Some(version), Some(policy)) => Some(Filter::and([version, policy])),
            (version, policy) => version.or(policy),
        }
    }

    /// Why an update of record `id` these guards filtered changed nothing: it's gone, its
    /// version moved on, or the policies don't let the actor change it.
    pub(crate) async fn unchanged<D: crate::data_layer::DataLayer>(
        &self,
        ctx: &crate::context::Context<D>,
        resource: &'static ResourceDef,
        id: Value,
    ) -> Error {
        let Ok(pk) = pk_name(resource) else {
            return Error::NotFound;
        };
        let Ok((filter, tenant)) =
            crate::pipeline::apply_tenant_scope(resource, Some(Filter::eq(pk, id.clone())), ctx.tenant.clone())
        else {
            return Error::NotFound;
        };
        let query = crate::data_layer::CompiledQuery { filter, tenant, limit: Some(1), ..Default::default() };
        let stored = match ctx.data.run_query(resource, &query).await {
            Ok(rows) => rows.into_iter().next(),
            Err(err) => return err,
        };
        match (stored, self.version) {
            (None, _) => Error::NotFound,
            (Some(row), Some((field, expected))) if row.get(field).and_then(Value::as_int) != Some(expected) => {
                Error::StaleRecord { resource: resource.name, id }
            }
            (Some(_), _) if self.policy.is_some() => Error::Forbidden,
            (Some(_), _) => Error::NotFound,
        }
    }
}

pub(crate) async fn run_atomic_update<D: crate::data_layer::DataLayer>(
    ctx: &crate::context::Context<D>,
    resource: &'static ResourceDef,
    action: &ActionDef,
    id: Value,
    update: &AtomicUpdate,
    guards: Option<&Filter>,
    scope: Option<&Filter>,
) -> Result<Option<FieldMap>> {
    let mut query = atomic_query(ctx, resource, action, id, scope)?;
    query.filter = crate::pipeline::and_filters(query.filter, guards.cloned());
    Ok(ctx.data.update_atomic(resource, &query, update).await?.into_iter().next())
}

/// Deletes record `id` as `ctx` sees it, as one statement, as [`run_atomic_update`]
/// updates it: when none of `conditions` holds. Returns what it held, or `None` when
/// there's no such record.
pub(crate) async fn run_atomic_destroy<D: crate::data_layer::DataLayer>(
    ctx: &crate::context::Context<D>,
    resource: &'static ResourceDef,
    action: &ActionDef,
    id: Value,
    conditions: &[AtomicCondition],
    scope: Option<&Filter>,
) -> Result<Option<FieldMap>> {
    let query = atomic_query(ctx, resource, action, id, scope)?;
    Ok(ctx.data.destroy_atomic(resource, &query, conditions).await?.into_iter().next())
}

/// The read that finds the record an update or destroy of `action` changes: the one it
/// upgrades with, else the primary read.
pub(crate) fn finding_read(resource: &'static ResourceDef, action: &ActionDef) -> Result<Option<&'static ActionDef>> {
    match action.atomic_upgrade_with {
        Some(name) => resource.action(name).map(Some).ok_or_else(|| Error::UnknownAction {
            resource: resource.name,
            name: name.to_string(),
        }),
        None => Ok(resource.primary_read()),
    }
}

/// The records `actor` may read through `read`, the read that finds an update's or
/// destroy's record by id: its filters and its policies, as a filter. AshGraphql and
/// AshTypescript find the record so, running the update or destroy over a query of the
/// read action (`Ash.bulk_update(query, ...)`), whose policies filter what it finds: a
/// record the actor can't read isn't found. `None` where there's no read.
pub(crate) fn read_scope(resource: &'static ResourceDef, read: Option<&ActionDef>, actor: Option<&Actor>) -> Result<Option<Filter>> {
    let Some(read) = read else {
        return Ok(None);
    };
    let policies = crate::policy::compile_read_filter(resource, read, actor)?;
    Ok(Some(Filter::and(resource.read_filter(read).into_iter().chain(policies))))
}

/// The query selecting record `id` for an atomic statement of `action`: by its primary
/// key, in the context's tenant, within `scope`, the read that finds it; or, given none,
/// through the read `action` upgrades with.
fn atomic_query<D>(
    ctx: &crate::context::Context<D>,
    resource: &'static ResourceDef,
    action: &ActionDef,
    id: Value,
    scope: Option<&Filter>,
) -> Result<crate::data_layer::CompiledQuery> {
    let pk = pk_name(resource)?;
    let (filter, tenant) =
        crate::pipeline::apply_tenant_scope(resource, Some(Filter::eq(pk, id.clone())), ctx.tenant.clone())?;
    let filter = match scope {
        Some(scope) => crate::pipeline::and_filters(filter, Some(scope.clone())),
        None => {
            let read = finding_read(resource, action)?;
            crate::pipeline::and_filters(filter, read.and_then(|read| resource.read_filter(read)))
        }
    };
    Ok(crate::data_layer::CompiledQuery {
        filter,
        tenant,
        actor: ctx.actor.clone(),
        limit: Some(1),
        ..crate::data_layer::CompiledQuery::default()
    })
}

/// What a planned update starts from.
pub(crate) struct PlanInput<'a> {
    pub actor: Option<&'a Actor>,
    pub tenant: Option<&'a str>,
    /// Values set outright: the accepted input, or what a changeset already changes.
    pub sets: FieldMap,
    pub arguments: &'a FieldMap,
    /// The lock version the record must still have, and the record's id for the error.
    pub expected_version: Option<(Value, i64)>,
    /// Whether the action's after-action and after-transaction changes are this plan's
    /// to run: not when a changeset already holds them.
    pub collect_hooks: bool,
    /// Whether the data layer raises a condition's error within its statement, as Ash's
    /// data layers that can `expr_error`. One that can't runs no plan with conditions.
    pub can_raise: bool,
}

/// Plans `action` on `resource` as one statement: an update, or a soft destroy, which
/// Ash runs as an update. A hard destroy's plan sets nothing: it deletes when none of
/// its conditions holds. `Ok(Err(reason))` when it can't be planned; `Err` when it fails
/// whatever the record (a value of the wrong type, a validation of a value it sets, a
/// policy no record passes).
pub(crate) fn plan_update(
    resource: &'static ResourceDef,
    action: &'static ActionDef,
    input: PlanInput<'_>,
) -> Result<std::result::Result<AtomicPlan, String>> {
    if action.persist != PersistKind::DataLayer {
        return Ok(Err("the action is manual".into()));
    }
    let hard_destroy = action.kind == ActionKind::Destroy && !action.soft;
    if hard_destroy {
        // Cascades read the record's related records, and destroy them, before it goes.
        if !action.cascade_destroy.is_empty() {
            return Ok(Err("it cascades to related records".into()));
        }
        let deletes_related = resource.relationships.iter().any(|rel| {
            matches!(rel.kind, RelKind::HasMany | RelKind::HasOne) && rel.on_delete != OnDelete::Nothing
        });
        if deletes_related {
            return Ok(Err("its relationships act on related records when it's deleted".into()));
        }
    }
    let PlanInput { actor, tenant, mut sets, arguments, expected_version, collect_hooks, can_raise } = input;
    let mut guards = Guards::default();
    let lock = action.optimistic_lock();
    // The lock checks the version the record had when it was read: an update by id
    // hasn't read it, so reads it first, as Ash's optimistic lock can't run over a query.
    if lock.is_some() && expected_version.is_none() {
        return Ok(Err("its optimistic lock checks the version the record was read at".into()));
    }
    let updated_at = resource.timestamps.map(|(_, updated_at)| updated_at);
    // The lock version and `updated_at` are the plan's to set, from the stored record.
    sets.retain(|name, _| Some(name.as_str()) != lock && Some(name.as_str()) != updated_at);
    validate_given(resource, &mut sets)?;

    let mut update = AtomicUpdate::default();
    for (name, value) in sets {
        update.set(name, AtomicExpr::Value(value));
    }
    let mut after_actions = Vec::new();
    let mut after_transactions = Vec::new();

    for change in action.changes {
        match change {
            // The lock version moves below, after the conditions.
            Change::OptimisticLock { .. } => {}
            Change::SetAttribute { field, value } => update.set(*field, AtomicExpr::value(Value::from(*value))),
            Change::SetAttributeFn { field, value } => update.set(*field, AtomicExpr::Value(value())),
            Change::SetNewAttribute { field, value } => {
                let current = update.value_of(field);
                update.set(*field, AtomicExpr::Coalesce(vec![current, AtomicExpr::value(Value::from(*value))]));
            }
            Change::SetNewAttributeFn { field, value } => {
                let current = update.value_of(field);
                update.set(*field, AtomicExpr::Coalesce(vec![current, AtomicExpr::Value(value())]));
            }
            Change::RelateActor { field } => {
                let actor = actor.ok_or(Error::Forbidden)?;
                update.set(*field, AtomicExpr::value(actor.id));
            }
            Change::SetFromArgument { field, argument } => {
                if let Some(value) = arguments.get(*argument) {
                    update.set(*field, AtomicExpr::Value(value.clone()));
                }
            }
            Change::AfterAction(hook) => {
                if collect_hooks {
                    after_actions.push(Box::new(*hook) as DynamicAfterActionHook);
                }
            }
            Change::AfterTransaction(hook) => {
                if collect_hooks {
                    after_transactions.push(Box::new(*hook) as DynamicAfterTransactionHook);
                }
            }
            Change::AtomicUpdate { field, expr } => match atomic_from_expr(expr, arguments) {
                Some(expr) => update.set(*field, expr),
                None => return Ok(Err(format!("its update of `{field}` can't run in the data layer"))),
            },
            Change::BeforeAction(_) => return Ok(Err("it has a before_action hook".into())),
            Change::ManageRelationship { .. } => return Ok(Err("it manages relationships".into())),
            Change::Func(_) => return Ok(Err("it has a change function".into())),
            Change::Custom(custom) => {
                let context = AtomicContext { resource, action, actor, tenant, arguments, update: &update };
                match custom.atomic(&context) {
                    Atomic::Atomic { set, conditions } => {
                        for (field, expr) in set {
                            update.set(field, expr);
                        }
                        update.conditions.extend(conditions);
                    }
                    Atomic::NotAtomic(reason) => return Ok(Err(reason)),
                }
            }
        }
    }

    // Values the changes set outright are checked now, as those the input sets were.
    let mut known: FieldMap = update
        .set
        .iter()
        .filter_map(|(name, expr)| expr.known().map(|value| (name.clone(), value.clone())))
        .collect();
    validate_given(resource, &mut known)?;
    for (name, value) in known {
        update.set(name, AtomicExpr::Value(value));
    }

    // The write policies, against the record as stored, as Ash authorizes an atomic update.
    // Policies no record passes refuse it once its validations have passed on what it
    // sets, as Ash validates a changeset's input before authorizing it.
    let allowed = write_filter(resource, action, actor)?;
    if !matches!(allowed, Filter::True | Filter::False) {
        if can_raise {
            update.conditions.push(AtomicCondition::failing_with(
                AtomicExpr::not_true(AtomicExpr::Filter(allowed.clone())),
                || Error::Forbidden,
            ));
        } else {
            guards.policy = Some(allowed.clone());
        }
    }

    let mut failed = Vec::new();
    for validation in action.validations {
        let field = match validation {
            Validation::Present { field }
            | Validation::StringLength { field, .. }
            | Validation::OneOf { field, .. }
            | Validation::Numericality { field, .. } => *field,
            Validation::Func(_) => return Ok(Err("it has a validation function".into())),
            Validation::Custom(custom) => {
                let context = AtomicContext { resource, action, actor, tenant, arguments, update: &update };
                match custom.atomic(&context) {
                    Atomic::Atomic { set, conditions } => {
                        for (field, expr) in set {
                            update.set(field, expr);
                        }
                        update.conditions.extend(conditions);
                    }
                    Atomic::NotAtomic(reason) => return Ok(Err(reason)),
                }
                continue;
            }
        };
        let context = AtomicContext { resource, action, actor, tenant, arguments, update: &update };
        let value = context.value_of(field);
        match value.known() {
            // The value it'll hold is known: check it now, every one, as Ash reports
            // every validation a changeset fails.
            Some(value) => {
                if let Err(error) = check_builtin_validation(validation, Some(value)) {
                    failed.push(error);
                }
            }
            None => update.conditions.extend(builtin_conditions(resource, validation, value)),
        }
    }
    Error::collect(failed)?;

    if allowed == Filter::False {
        return Err(Error::Forbidden);
    }

    if let (Some(version), Some((id, expected))) = (lock, expected_version) {
        if can_raise {
            let resource_name = resource.name;
            update.conditions.push(AtomicCondition::failing_with(
                AtomicExpr::DistinctFrom(Box::new(AtomicExpr::field(version)), Box::new(AtomicExpr::value(expected))),
                move || Error::StaleRecord { resource: resource_name, id: id.clone() },
            ));
        } else {
            guards.version = Some((version, expected));
        }
    }

    // A data layer that can't raise in its statement can't check what fails there, as
    // Ash's validations aren't atomic on a data layer without `expr_error`.
    if !can_raise && !update.conditions.is_empty() {
        return Ok(Err("its validations or policies check the stored record, which the data layer can't raise errors from in its statement".into()));
    }

    if hard_destroy {
        // Nothing to set: the conditions decide whether it deletes.
        update.set.clear();
        return Ok(Ok(AtomicPlan { update, guards, after_actions, after_transactions }));
    }

    if let Some(version) = lock {
        let bumped = AtomicExpr::Add(Box::new(AtomicExpr::field(version)), Box::new(AtomicExpr::value(1i64)));
        update.set(version, bumped);
    }

    // `updated_at` moves only when a value does, as Ash's update timestamp.
    if let Some(updated_at) = updated_at {
        let changed: Vec<AtomicExpr> = update
            .set
            .iter()
            .filter(|(name, _)| Some(name.as_str()) != lock)
            .map(|(name, expr)| AtomicExpr::DistinctFrom(Box::new(AtomicExpr::field(name.clone())), Box::new(expr.clone())))
            .collect();
        if !changed.is_empty() {
            let now = Value::String(crate::types::UtcDateTimeUsec::now().as_str().to_string());
            update.set(
                updated_at,
                AtomicExpr::If {
                    condition: Box::new(AtomicExpr::Or(changed)),
                    then: Box::new(AtomicExpr::Value(now)),
                    otherwise: Box::new(AtomicExpr::field(updated_at)),
                },
            );
        }
    }

    // An update that sets nothing still checks its conditions and returns the record.
    if update.set.is_empty() {
        let pk = pk_name(resource)?;
        update.set(pk, AtomicExpr::field(pk));
    }

    Ok(Ok(AtomicPlan { update, guards, after_actions, after_transactions }))
}

/// When a built-in validation of a value the statement computes fails, as Ash's
/// validations do atomically: a condition for each way it fails, with the error the
/// record-by-record check gives. Nil passes, as it does record by record.
fn builtin_conditions(resource: &ResourceDef, validation: &Validation, value: AtomicExpr) -> Vec<AtomicCondition> {
    let text = match &value {
        AtomicExpr::Field(name) => resource
            .attribute(name)
            .is_some_and(|attr| matches!(attr.ty, AttrType::String | AttrType::CiString)),
        _ => false,
    };
    // Each fails with the validation's own error, as Ash describes it.
    let builtin = *validation;
    let condition = move |fails_when: AtomicExpr| {
        AtomicCondition::failing_with(fails_when, move || builtin.error().expect("a built-in validation"))
    };
    let boxed = Box::new;
    match validation {
        Validation::Present { .. } => {
            let blank = if text {
                AtomicExpr::Or(vec![
                    AtomicExpr::IsNil(boxed(value.clone())),
                    AtomicExpr::Eq(boxed(AtomicExpr::Trim(boxed(value))), boxed(AtomicExpr::value(""))),
                ])
            } else {
                AtomicExpr::IsNil(boxed(value))
            };
            vec![condition(blank)]
        }
        Validation::StringLength { min, max, .. } if text => {
            let length = || boxed(AtomicExpr::StringLength(boxed(value.clone())));
            let mut conditions = Vec::new();
            if let Some(min) = min {
                conditions.push(condition(AtomicExpr::Lt(length(), boxed(AtomicExpr::value(*min as i64)))));
            }
            if let Some(max) = max {
                conditions.push(condition(AtomicExpr::Gt(length(), boxed(AtomicExpr::value(*max as i64)))));
            }
            conditions
        }
        Validation::OneOf { allowed, .. } => vec![condition(AtomicExpr::Not(boxed(AtomicExpr::In(
            boxed(value),
            allowed.iter().map(|v| Value::String((*v).to_string())).collect(),
        ))))],
        Validation::Numericality { min, max, .. } => {
            let mut conditions = Vec::new();
            if let Some(min) = min {
                conditions.push(condition(AtomicExpr::Lt(boxed(value.clone()), boxed(AtomicExpr::value(*min)))));
            }
            if let Some(max) = max {
                conditions.push(condition(AtomicExpr::Gt(boxed(value.clone()), boxed(AtomicExpr::value(*max)))));
            }
            conditions
        }
        Validation::StringLength { .. } | Validation::Custom(_) | Validation::Func(_) => Vec::new(),
    }
}

/// `expr`, a resource expression, as an atomic one over the record as stored, its
/// arguments' values known: `None` where it calls Rust, which a statement can't.
pub(crate) fn atomic_from_expr(expr: &crate::expr::Expr, arguments: &FieldMap) -> Option<AtomicExpr> {
    use crate::expr::Expr;
    let of = |e: &Expr| atomic_from_expr(e, arguments);
    let both = |a: &Expr, b: &Expr| Some((Box::new(of(a)?), Box::new(of(b)?)));
    Some(match *expr {
        Expr::Field(name) => AtomicExpr::field(name),
        Expr::Arg(name) => AtomicExpr::Value(arguments.get(name).cloned().unwrap_or(Value::Null)),
        Expr::LitInt(n) => AtomicExpr::value(n),
        Expr::LitString(s) => AtomicExpr::value(s.to_string()),
        Expr::LitBool(b) => AtomicExpr::value(b),
        Expr::Null => AtomicExpr::Value(Value::Null),
        Expr::StringLength(name) => AtomicExpr::StringLength(Box::new(AtomicExpr::field(name))),
        Expr::Length(e) => AtomicExpr::StringLength(Box::new(of(e)?)),
        Expr::Lower(e) => AtomicExpr::Lower(Box::new(of(e)?)),
        Expr::Upper(e) => AtomicExpr::Upper(Box::new(of(e)?)),
        Expr::Concat(parts) => AtomicExpr::Concat(parts.iter().map(|e| of(e)).collect::<Option<_>>()?),
        Expr::Coalesce(parts) => AtomicExpr::Coalesce(parts.iter().map(|e| of(e)).collect::<Option<_>>()?),
        Expr::Add(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Add(a, b)
        }
        Expr::Sub(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Sub(a, b)
        }
        Expr::Mul(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Mul(a, b)
        }
        Expr::Div(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Div(a, b)
        }
        Expr::Eq(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Eq(a, b)
        }
        Expr::Ne(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Not(Box::new(AtomicExpr::Eq(a, b)))
        }
        Expr::Gt(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Gt(a, b)
        }
        Expr::Lt(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Lt(a, b)
        }
        Expr::Gte(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Not(Box::new(AtomicExpr::Lt(a, b)))
        }
        Expr::Lte(a, b) => {
            let (a, b) = both(a, b)?;
            AtomicExpr::Not(Box::new(AtomicExpr::Gt(a, b)))
        }
        Expr::IfElse { cond, then_expr, else_expr } => AtomicExpr::If {
            condition: Box::new(of(cond)?),
            then: Box::new(of(then_expr)?),
            otherwise: Box::new(of(else_expr)?),
        },
        Expr::Custom(_) => return None,
    })
}
