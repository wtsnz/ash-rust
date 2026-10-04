//! Atomic updates, as Ash runs them: an update written as one statement in the data layer,
//! rather than read, changed in memory and written back. Each change says what it sets as
//! an expression over the record as the data layer holds it, and each validation (and the
//! write policies, and a state machine's transition) says when the update must fail. The
//! data layer evaluates all of it in the statement, against the record as it is then, so
//! a concurrent write can't slip between a read and the update.
//!
//! An action runs atomically when everything in it can: `require_atomic` (true by
//! default, as in Ash) makes an action that can't fail rather than fall back to reading
//! first, where its data layer could have run it atomically.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use crate::actor::Actor;
use crate::error::Error;
use crate::filter::Filter;
use crate::resource::ResourceDef;
use crate::value::{FieldMap, Value};

/// A value an atomic update computes from the record as the data layer holds it.
#[derive(Clone, Debug, PartialEq)]
pub enum AtomicExpr {
    /// A value known before the statement runs: input, an argument, the actor.
    Value(Value),
    /// The record's attribute, as stored when the statement runs.
    Field(String),
    Add(Box<AtomicExpr>, Box<AtomicExpr>),
    Sub(Box<AtomicExpr>, Box<AtomicExpr>),
    Mul(Box<AtomicExpr>, Box<AtomicExpr>),
    /// Division, integer division for integers, as SQL's.
    Div(Box<AtomicExpr>, Box<AtomicExpr>),
    Lower(Box<AtomicExpr>),
    Upper(Box<AtomicExpr>),
    /// Text joined end to end; nil if any part is.
    Concat(Vec<AtomicExpr>),
    /// The length of text, in characters.
    StringLength(Box<AtomicExpr>),
    /// Text without leading and trailing whitespace.
    Trim(Box<AtomicExpr>),
    /// The first of these that isn't nil.
    Coalesce(Vec<AtomicExpr>),
    If {
        condition: Box<AtomicExpr>,
        then: Box<AtomicExpr>,
        otherwise: Box<AtomicExpr>,
    },
    IsNil(Box<AtomicExpr>),
    Eq(Box<AtomicExpr>, Box<AtomicExpr>),
    Lt(Box<AtomicExpr>, Box<AtomicExpr>),
    Gt(Box<AtomicExpr>, Box<AtomicExpr>),
    /// Unequal, where nil equals nil and nothing else (SQL's `IS DISTINCT FROM`).
    DistinctFrom(Box<AtomicExpr>, Box<AtomicExpr>),
    /// One of these values.
    In(Box<AtomicExpr>, Vec<Value>),
    And(Vec<AtomicExpr>),
    Or(Vec<AtomicExpr>),
    Not(Box<AtomicExpr>),
    /// A filter over the record, such as a policy's.
    Filter(Filter),
}

impl AtomicExpr {
    pub fn value(value: impl Into<Value>) -> Self {
        Self::Value(value.into())
    }

    pub fn field(name: impl Into<String>) -> Self {
        Self::Field(name.into())
    }

    /// Holds unless `allowed` is true: false and nil both fail it, as a policy that isn't
    /// met forbids, whether its check is false or can't be decided.
    pub fn not_true(allowed: AtomicExpr) -> Self {
        Self::Not(Box::new(Self::Coalesce(vec![allowed, Self::Value(Value::Bool(false))])))
    }

    /// The value, if it's known before the statement runs.
    pub fn known(&self) -> Option<&Value> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }

    /// Evaluates the expression against `row`, as the data layer would: SQL's nil
    /// propagation, with a nil condition counting as not holding. `filter` evaluates a
    /// [`Filter`](Self::Filter), as the data layer filters its records.
    pub fn eval(&self, resource: &ResourceDef, row: &FieldMap, filter: &dyn Fn(&Filter, &FieldMap) -> bool) -> Value {
        let eval = |e: &AtomicExpr| e.eval(resource, row, filter);
        let truth = |e: &AtomicExpr| match eval(e) {
            Value::Bool(b) => Some(b),
            _ => None,
        };
        let compare = |a: &AtomicExpr, b: &AtomicExpr| -> Option<Ordering> {
            let (x, y) = (eval(a), eval(b));
            if x.is_null() || y.is_null() {
                return None;
            }
            let ty = [a, b].into_iter().find_map(|e| match e {
                Self::Field(name) => resource.attribute(name).map(|attr| attr.ty),
                _ => None,
            });
            Some(crate::types::compare_typed(ty, &x, &y))
        };
        let bool_or_null = |b: Option<bool>| b.map(Value::Bool).unwrap_or(Value::Null);
        match self {
            Self::Value(value) => value.clone(),
            Self::Field(name) => row.get(name).cloned().unwrap_or(Value::Null),
            Self::Add(a, b) | Self::Sub(a, b) | Self::Mul(a, b) | Self::Div(a, b) => match (eval(a), eval(b)) {
                (Value::Int(x), Value::Int(y)) => match self {
                    Self::Add(..) => Value::Int(x + y),
                    Self::Sub(..) => Value::Int(x - y),
                    Self::Mul(..) => Value::Int(x * y),
                    _ if y == 0 => Value::Null,
                    _ => Value::Int(x / y),
                },
                _ => Value::Null,
            },
            Self::Lower(e) => match eval(e) {
                Value::String(s) => Value::String(s.to_lowercase()),
                _ => Value::Null,
            },
            Self::Upper(e) => match eval(e) {
                Value::String(s) => Value::String(s.to_uppercase()),
                _ => Value::Null,
            },
            Self::Concat(parts) => {
                let mut out = String::new();
                for part in parts {
                    match eval(part) {
                        Value::Null => return Value::Null,
                        Value::String(s) => out.push_str(&s),
                        other => out.push_str(&other.to_string()),
                    }
                }
                Value::String(out)
            }
            Self::StringLength(e) => match eval(e) {
                Value::String(s) => Value::Int(s.chars().count() as i64),
                _ => Value::Null,
            },
            Self::Trim(e) => match eval(e) {
                Value::String(s) => Value::String(s.trim().to_string()),
                _ => Value::Null,
            },
            Self::Coalesce(items) => items.iter().map(eval).find(|v| !v.is_null()).unwrap_or(Value::Null),
            Self::If { condition, then, otherwise } => {
                if truth(condition) == Some(true) {
                    eval(then)
                } else {
                    eval(otherwise)
                }
            }
            Self::IsNil(e) => Value::Bool(eval(e).is_null()),
            Self::Eq(a, b) => bool_or_null(compare(a, b).map(|o| o == Ordering::Equal)),
            Self::Lt(a, b) => bool_or_null(compare(a, b).map(|o| o == Ordering::Less)),
            Self::Gt(a, b) => bool_or_null(compare(a, b).map(|o| o == Ordering::Greater)),
            Self::DistinctFrom(a, b) => {
                let (x, y) = (eval(a), eval(b));
                Value::Bool(match (x.is_null(), y.is_null()) {
                    (true, true) => false,
                    (true, false) | (false, true) => true,
                    _ => compare(a, b) != Some(Ordering::Equal),
                })
            }
            Self::In(e, values) => {
                let x = eval(e);
                if x.is_null() {
                    Value::Null
                } else {
                    let ty = match e.as_ref() {
                        Self::Field(name) => resource.attribute(name).map(|attr| attr.ty),
                        _ => None,
                    };
                    Value::Bool(values.iter().any(|v| crate::types::compare_typed(ty, &x, v) == Ordering::Equal))
                }
            }
            Self::And(items) => {
                let results: Vec<Option<bool>> = items.iter().map(truth).collect();
                if results.contains(&Some(false)) {
                    Value::Bool(false)
                } else if results.contains(&None) {
                    Value::Null
                } else {
                    Value::Bool(true)
                }
            }
            Self::Or(items) => {
                let results: Vec<Option<bool>> = items.iter().map(truth).collect();
                if results.contains(&Some(true)) {
                    Value::Bool(true)
                } else if results.contains(&None) {
                    Value::Null
                } else {
                    Value::Bool(false)
                }
            }
            Self::Not(e) => bool_or_null(truth(e).map(|b| !b)),
            Self::Filter(f) => Value::Bool(filter(f, row)),
        }
    }
}

/// When an atomic update must fail, and the error it fails with.
#[derive(Clone)]
pub struct AtomicCondition {
    /// The update fails when this holds for the record as the data layer holds it.
    pub fails_when: AtomicExpr,
    /// Attributes of the record the error reports, read in the same statement.
    pub reports: Vec<String>,
    /// The error, from the reported attributes.
    pub error: Arc<dyn Fn(&FieldMap) -> Error + Send + Sync>,
}

impl AtomicCondition {
    pub fn new(
        fails_when: AtomicExpr,
        reports: Vec<String>,
        error: impl Fn(&FieldMap) -> Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            fails_when,
            reports,
            error: Arc::new(error),
        }
    }

    /// A condition whose error doesn't depend on the record.
    pub fn failing_with(fails_when: AtomicExpr, error: impl Fn() -> Error + Send + Sync + 'static) -> Self {
        Self::new(fails_when, Vec::new(), move |_| error())
    }
}

impl fmt::Debug for AtomicCondition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AtomicCondition")
            .field("fails_when", &self.fails_when)
            .field("reports", &self.reports)
            .finish()
    }
}

/// An update as one statement: what it sets, and when it fails. The data layer checks
/// every condition, in order, against each record the update's query selects, before
/// setting anything; every expression reads the record as it was before the update.
#[derive(Clone, Debug, Default)]
pub struct AtomicUpdate {
    pub set: Vec<(String, AtomicExpr)>,
    pub conditions: Vec<AtomicCondition>,
}

impl AtomicUpdate {
    /// What `field` will hold after the update: what it's set to, or what it holds.
    pub fn value_of(&self, field: &str) -> AtomicExpr {
        self.set
            .iter()
            .rev()
            .find(|(name, _)| name == field)
            .map(|(_, expr)| expr.clone())
            .unwrap_or_else(|| AtomicExpr::field(field))
    }

    /// Sets `field` to `expr`, replacing what an earlier change set it to.
    pub fn set(&mut self, field: impl Into<String>, expr: AtomicExpr) {
        let field = field.into();
        self.set.retain(|(name, _)| *name != field);
        self.set.push((field, expr));
    }

    /// Checks the conditions and applies the update to `row`, as a data layer that holds
    /// its records in memory does. On success, returns the updated record.
    pub fn apply(
        &self,
        resource: &ResourceDef,
        row: &FieldMap,
        filter: &dyn Fn(&Filter, &FieldMap) -> bool,
    ) -> Result<FieldMap, Error> {
        for condition in &self.conditions {
            if condition.fails_when.eval(resource, row, filter) == Value::Bool(true) {
                return Err((condition.error)(row));
            }
        }
        let values: Vec<(String, Value)> = self
            .set
            .iter()
            .map(|(name, expr)| (name.clone(), expr.eval(resource, row, filter)))
            .collect();
        let mut updated = row.clone();
        updated.extend(values);
        Ok(updated)
    }
}

/// What a change, a validation or a policy contributes to an atomic update.
#[derive(Clone, Debug)]
pub enum Atomic {
    /// It can run in the statement: values to set, and conditions to check.
    Atomic {
        set: Vec<(String, AtomicExpr)>,
        conditions: Vec<AtomicCondition>,
    },
    /// It can't: why not.
    NotAtomic(String),
}

impl Atomic {
    /// Contributes nothing: it holds whatever the record.
    pub fn nothing() -> Self {
        Self::Atomic {
            set: Vec::new(),
            conditions: Vec::new(),
        }
    }

    pub fn conditions(conditions: Vec<AtomicCondition>) -> Self {
        Self::Atomic {
            set: Vec::new(),
            conditions,
        }
    }

    pub fn not_atomic(reason: impl Into<String>) -> Self {
        Self::NotAtomic(reason.into())
    }
}

/// What a custom change or validation sees when asked how it runs atomically.
pub struct AtomicContext<'a> {
    pub resource: &'static ResourceDef,
    pub action: &'static crate::action::ActionDef,
    pub actor: Option<&'a Actor>,
    pub tenant: Option<&'a str>,
    pub arguments: &'a FieldMap,
    /// The update as planned so far: what earlier changes set.
    pub update: &'a AtomicUpdate,
}

impl AtomicContext<'_> {
    /// What `field` will hold after the update, as far as the update is planned.
    pub fn value_of(&self, field: &str) -> AtomicExpr {
        match self.arguments.get(field) {
            Some(value) if self.resource.attribute(field).is_none() => AtomicExpr::Value(value.clone()),
            _ => self.update.value_of(field),
        }
    }
}
