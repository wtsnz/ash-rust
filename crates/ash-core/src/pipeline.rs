use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, Change, PersistKind, Validation};
use crate::actor::Actor;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::{AttrType, AttributeDef, ResourceDef};
use crate::value::{FieldMap, Value};

pub fn action_named<'a>(def: &'a ResourceDef, name: &str) -> Result<&'a ActionDef> {
    def.action(name).ok_or_else(|| Error::UnknownAction {
        resource: def.name,
        name: name.to_string(),
    })
}

pub fn read_action<'a>(def: &'a ResourceDef, name: Option<&str>) -> Result<&'a ActionDef> {
    let action = match name {
        Some(name) => action_named(def, name)?,
        None => def.default_read(),
    };
    expect_kind(action, ActionKind::Read)?;
    Ok(action)
}

pub fn expect_kind(action: &ActionDef, expected: ActionKind) -> Result<()> {
    if action.kind == expected {
        Ok(())
    } else {
        Err(Error::WrongActionKind {
            action: action.name,
            expected: expected.as_str(),
            actual: action.kind.as_str(),
        })
    }
}

pub fn expect_persist(action: &ActionDef, expected: PersistKind) -> Result<()> {
    if action.persist == expected {
        Ok(())
    } else if expected == PersistKind::Manual {
        Err(Error::NotManual {
            action: action.name,
        })
    } else {
        Err(Error::ManualRequired {
            action: action.name,
        })
    }
}

pub fn pk_name(def: &ResourceDef) -> Result<&'static str> {
    def.primary_key()
        .map(|attribute| attribute.name)
        .ok_or(Error::NoPrimaryKey(def.name))
}

pub fn split_input(resource: &ResourceDef, action: &ActionDef, input: FieldMap) -> Result<(FieldMap, FieldMap)> {
    let mut fields = FieldMap::new();
    let mut arguments = FieldMap::new();
    for (field, mut value) in input {
        if action.accept.contains(&field.as_str()) {
            if let Some(attribute) = resource.attribute(&field) {
                cast_text(attribute.ty, &mut value);
            }
            fields.insert(field, value);
        } else if let Some(arg) = action.arguments.iter().find(|arg| arg.name == field) {
            cast_text(arg.ty, &mut value);
            arguments.insert(field, value);
        } else {
            return Err(Error::NotAccepted {
                field,
                action: action.name,
            });
        }
    }
    crate::action::apply_argument_defaults(action.arguments, &mut arguments);
    for arg in action.arguments {
        if !arg.allow_nil && arguments.get(arg.name).is_none_or(Value::is_null) {
            return Err(Error::Missing {
                field: arg.name.to_string(),
            });
        }
    }
    Ok((fields, arguments))
}

/// Text as Ash's string types cast it by default: trimmed, and nil when that leaves
/// nothing (`trim?: true`, `allow_empty?: false`).
pub(crate) fn cast_text(ty: AttrType, value: &mut Value) {
    if !matches!(ty, AttrType::String | AttrType::CiString) {
        return;
    }
    if let Value::String(text) = value {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            *value = Value::Null;
        } else if trimmed.len() != text.len() {
            *text = trimmed.to_string();
        }
    }
}


pub fn generate_pk(def: &ResourceDef, fields: &mut FieldMap) {
    if let Some(pk) = def.primary_key()
        && pk.generated
        && !fields.contains_key(pk.name)
    {
        fields.insert(pk.name.to_string(), Value::Uuid(Uuid::new_v4()));
    }
}

pub fn apply_changes(
    fields: &mut FieldMap,
    action: &ActionDef,
    actor: Option<&Actor>,
    arguments: &FieldMap,
) -> Result<()> {
    let mut before_actions = Vec::new();
    let mut after_actions = Vec::new();
    let mut after_transactions = Vec::new();
    apply_changes_with_context(
        fields,
        action,
        actor,
        None,
        &FieldMap::new(),
        arguments,
        &mut before_actions,
        &mut after_actions,
        &mut after_transactions,
    )
}

#[allow(dead_code)]
pub fn apply_changes_with_hooks(
    fields: &mut FieldMap,
    action: &ActionDef,
    actor: Option<&Actor>,
    arguments: &FieldMap,
    before_actions: &mut Vec<crate::action::DynamicBeforeActionHook>,
    after_actions: &mut Vec<crate::action::DynamicAfterActionHook>,
    after_transactions: &mut Vec<crate::action::DynamicAfterTransactionHook>,
) -> Result<()> {
    apply_changes_with_context(
        fields,
        action,
        actor,
        None,
        &FieldMap::new(),
        arguments,
        before_actions,
        after_actions,
        after_transactions,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn apply_changes_with_context(
    fields: &mut FieldMap,
    action: &ActionDef,
    actor: Option<&Actor>,
    tenant: Option<&str>,
    metadata: &FieldMap,
    arguments: &FieldMap,
    before_actions: &mut Vec<crate::action::DynamicBeforeActionHook>,
    after_actions: &mut Vec<crate::action::DynamicAfterActionHook>,
    after_transactions: &mut Vec<crate::action::DynamicAfterTransactionHook>,
) -> Result<()> {
    for change in action.changes {
        match change {
            Change::SetAttribute { field, value } => {
                fields.insert((*field).to_string(), Value::from(*value));
            }
            Change::SetNewAttribute { field, value } => match fields.get(*field) {
                None | Some(Value::Null) => {
                    fields.insert((*field).to_string(), Value::from(*value));
                }
                _ => {}
            },
            Change::RelateActor { field } => {
                let actor = actor.ok_or(Error::Forbidden)?;
                fields.insert((*field).to_string(), Value::Uuid(actor.id));
            }
            Change::SetFromArgument { field, argument } => {
                if let Some(val) = arguments.get(*argument) {
                    fields.insert((*field).to_string(), val.clone());
                }
            }
            Change::SetAttributeFn { field, value } => {
                fields.insert((*field).to_string(), value());
            }
            Change::SetNewAttributeFn { field, value } => match fields.get(*field) {
                None | Some(Value::Null) => {
                    fields.insert((*field).to_string(), value());
                }
                _ => {}
            },
            Change::BeforeAction(hook) => {
                before_actions.push(Box::new(*hook));
            }
            Change::AfterAction(hook) => {
                after_actions.push(Box::new(*hook));
            }
            Change::AfterTransaction(hook) => {
                after_transactions.push(Box::new(*hook));
            }
            Change::Custom(c) => {
                let mut ctx = crate::action::ChangeContext {
                    fields,
                    actor,
                    tenant,
                    metadata,
                    arguments,
                    before_actions,
                    after_actions,
                    after_transactions,
                };
                c.apply(&mut ctx)?;
            }
            Change::ManageRelationship { .. } => {}
            Change::AtomicUpdate { field, expr } => {
                let value = crate::expr::eval_with_args(expr, fields, arguments)?;
                fields.insert((*field).to_string(), value);
            }
            Change::Func(f) => {
                let mut ctx = crate::action::ChangeContext {
                    fields,
                    actor,
                    tenant,
                    metadata,
                    arguments,
                    before_actions,
                    after_actions,
                    after_transactions,
                };
                f(&mut ctx)?;
            }
        }
    }
    Ok(())
}

pub fn run_validations(
    def: &ResourceDef,
    action: &ActionDef,
    record: Option<&FieldMap>,
    fields: &FieldMap,
    arguments: &FieldMap,
) -> Result<()> {
    run_validations_with_context(
        def,
        action,
        record,
        fields,
        None,
        None,
        &FieldMap::new(),
        arguments,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_validations_with_context(
    _def: &ResourceDef,
    action: &ActionDef,
    record: Option<&FieldMap>,
    fields: &FieldMap,
    actor: Option<&Actor>,
    tenant: Option<&str>,
    metadata: &FieldMap,
    arguments: &FieldMap,
) -> Result<()> {
    let get_val = |f: &str| fields.get(f).or_else(|| arguments.get(f));
    // Every validation runs, and every failure is reported, as Ash's are.
    let mut errors = Vec::new();
    for validation in action.validations {
        let ctx = crate::action::ValidationContext { record, fields, actor, tenant, metadata, arguments };
        let result = match validation {
            Validation::Present { field }
            | Validation::StringLength { field, .. }
            | Validation::OneOf { field, .. }
            | Validation::Numericality { field, .. } => check_builtin_validation(validation, get_val(field)),
            Validation::Custom(c) => c.validate(&ctx),
            Validation::Func(f) => f(&ctx),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    Error::collect(errors)
}

/// Checks a built-in validation (`present`, `string_length`, `one_of`, `numericality`)
/// against the value it validates, as its field will hold it.
pub(crate) fn check_builtin_validation(validation: &Validation, value: Option<&Value>) -> Result<()> {
    let fails = match *validation {
        Validation::Present { .. } => match value {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => s.trim().is_empty(),
            _ => false,
        },
        Validation::StringLength { min, max, .. } => match value {
            Some(Value::String(s)) => {
                let length = s.chars().count();
                min.is_some_and(|min| length < min) || max.is_some_and(|max| length > max)
            }
            _ => false,
        },
        Validation::OneOf { allowed, .. } => matches!(value, Some(Value::String(s)) if !allowed.contains(&s.as_str())),
        Validation::Numericality { field, min, max } => {
            // Integers, and floats and decimals stored as their text, as Ash's
            // numericality checks every kind of number.
            let number = match value {
                None | Some(Value::Null) => None,
                Some(Value::Int(n)) => Some(*n as f64),
                Some(Value::String(text)) => match text.parse::<f64>() {
                    Ok(n) if n.is_finite() => Some(n),
                    _ => return Err(Error::validation(field, "must be a number", Vec::new())),
                },
                Some(_) => return Err(Error::validation(field, "must be a number", Vec::new())),
            };
            number.is_some_and(|n| min.is_some_and(|min| n < min as f64) || max.is_some_and(|max| n > max as f64))
        }
        Validation::Custom(_) | Validation::Func(_) => false,
    };
    match validation.error() {
        Some(error) if fails => Err(error),
        _ => Ok(()),
    }
}

pub fn and_filters(left: Option<Filter>, right: Option<Filter>) -> Option<Filter> {
    match (left, right) {
        (None, None) => None,
        (Some(filter), None) | (None, Some(filter)) => Some(filter),
        (Some(left), Some(right)) => Some(Filter::and([left, right])),
    }
}

/// AND a tenant attribute filter onto a query, and require a tenant when the resource is not global.
pub fn apply_tenant_scope(
    resource: &ResourceDef,
    filter: Option<Filter>,
    tenant: Option<String>,
) -> Result<(Option<Filter>, Option<String>)> {
    if let Some(mt) = resource.multitenancy
        && tenant.is_none()
        && !mt.global
    {
        return Err(Error::TenantRequired {
            resource: resource.name,
        });
    }
    let filter = and_filters(filter, resource.tenant_filter(tenant.as_deref()));
    Ok((filter, tenant))
}

/// [`apply_tenant_scope`] plus the primary read's filters, so a lookup by id for a write
/// sees the same records a read does. An archived record stays out of reach.
pub fn visible_scope(
    resource: &ResourceDef,
    filter: Option<Filter>,
    tenant: Option<String>,
) -> Result<(Option<Filter>, Option<String>)> {
    let (filter, tenant) = apply_tenant_scope(resource, filter, tenant)?;
    Ok((
        and_filters(filter, resource.primary_read_filter()),
        tenant,
    ))
}

/// Stamp or require a tenant on write fields. Attribute strategy writes `tenant` onto `fields`.
pub fn apply_tenant_to_fields(
    resource: &ResourceDef,
    fields: &mut FieldMap,
    tenant: Option<&str>,
    for_create: bool,
) -> Result<()> {
    if let Some(mt) = resource.multitenancy {
        match mt.strategy {
            crate::resource::MultitenancyStrategy::Attribute(attr_name) => {
                if let Some(tenant) = tenant {
                    if for_create {
                        fields.insert(attr_name.to_string(), Value::String(tenant.to_string()));
                    }
                } else if !mt.global {
                    return Err(Error::TenantRequired {
                        resource: resource.name,
                    });
                }
            }
            crate::resource::MultitenancyStrategy::Context => {
                if tenant.is_none() && !mt.global {
                    return Err(Error::TenantRequired {
                        resource: resource.name,
                    });
                }
            }
        }
    }
    Ok(())
}

/// Split accepted attributes from action arguments. Extra keys (pk, version) are ignored
/// so GraphQL can pass `id` on the same map. An empty accept list keeps non-argument keys
/// as fields, matching the GraphQL input builder.
/// Checks each attribute's value against its type, then rewrites values with several
/// spellings (IP addresses, vectors, floats) to their canonical text.
pub fn validate(def: &ResourceDef, fields: &mut FieldMap) -> Result<()> {
    for attribute in def.attributes {
        match fields.get_mut(attribute.name) {
            None | Some(Value::Null) if attribute.allow_nil => {}
            None | Some(Value::Null) => {
                return Err(Error::Missing {
                    field: attribute.name.to_string(),
                });
            }
            Some(value) => {
                check_type(attribute, value)?;
                if let Value::String(raw) = value
                    && let Some(canonical) = crate::types::canonical_text(attribute.ty, raw)
                {
                    *raw = canonical;
                }
            }
        }
    }
    Ok(())
}

/// [`validate`] for only the attributes `fields` holds: what an update sets.
pub(crate) fn validate_given(def: &ResourceDef, fields: &mut FieldMap) -> Result<()> {
    for attribute in def.attributes {
        match fields.get_mut(attribute.name) {
            None => {}
            Some(Value::Null) if attribute.allow_nil => {}
            Some(Value::Null) => {
                return Err(Error::Missing {
                    field: attribute.name.to_string(),
                });
            }
            Some(value) => {
                check_type(attribute, value)?;
                if let Value::String(raw) = value
                    && let Some(canonical) = crate::types::canonical_text(attribute.ty, raw)
                {
                    *raw = canonical;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn check_type(attribute: &AttributeDef, value: &Value) -> Result<()> {
    let ok = match (attribute.ty, value) {
        (AttrType::Uuid, Value::Uuid(_))
        | (AttrType::String, Value::String(_))
        | (AttrType::Integer, Value::Int(_))
        | (AttrType::Boolean, Value::Bool(_))
        | (AttrType::Map, Value::Map(_))
        | (AttrType::Array { .. }, Value::Array(_)) => true,
        (AttrType::UtcDatetime { precision }, Value::String(got)) => match precision.normalize(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be an RFC3339 timestamp, got {got}"),
                });
            }
        },
        (AttrType::Decimal, Value::String(got)) => match crate::Decimal::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be a decimal number, got {got}"),
                });
            }
        },
        (AttrType::Binary, Value::String(got)) => match crate::Binary::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be base64, got {got}"),
                });
            }
        },
        (AttrType::Date, Value::String(got)) => match crate::Date::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be a calendar date, got {got}"),
                });
            }
        },
        (AttrType::Inet, Value::String(got)) => match crate::Inet::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be an IP address, got {got}"),
                });
            }
        },
        (AttrType::Vector { dimensions }, Value::String(got)) => {
            match crate::parse_vector(got).and_then(|v| crate::check_vector(&v, dimensions)) {
                Ok(()) => true,
                Err(err) => {
                    return Err(Error::Constraint {
                        field: attribute.name.to_string(),
                        message: err.to_string(),
                    });
                }
            }
        }
        (AttrType::CiString, Value::String(got)) => match crate::CiString::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be a string, got {got}"),
                });
            }
        },
        (AttrType::Float, Value::String(got)) => match crate::Float::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be a finite float, got {got}"),
                });
            }
        },
        (AttrType::Atom { one_of, .. }, Value::String(got)) => {
            if one_of.contains(&got.as_str()) {
                true
            } else {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be one of {one_of:?}"),
                });
            }
        }
        _ => false,
    };

    if ok {
        Ok(())
    } else {
        Err(Error::TypeMismatch {
            field: attribute.name.to_string(),
            expected: attribute.ty.name().into(),
            got: value.type_name().into(),
        })
    }
}

/// Values a new record gets before its action's changes run: a generated primary key,
/// the first lock version, attribute defaults, and timestamps.
pub(crate) fn prepare_create_fields(def: &ResourceDef, fields: &mut FieldMap) {
    let missing =
        |fields: &FieldMap, name: &str| matches!(fields.get(name), None | Some(Value::Null));
    generate_pk(def, fields);
    if let Some(version) = def.optimistic_lock_attribute()
        && missing(fields, version)
    {
        fields.insert(version.to_string(), Value::Int(1));
    }
    for attr in def.attributes {
        if let Some(default) = attr.default_fn
            && missing(fields, attr.name)
        {
            fields.insert(attr.name.to_string(), default());
        }
    }
    if let Some((created_at, updated_at)) = def.timestamps {
        let now = crate::types::UtcDateTimeUsec::now().as_str().to_string();
        for name in [created_at, updated_at] {
            if missing(fields, name) {
                fields.insert(name.to_string(), Value::String(now.clone()));
            }
        }
    }
}

/// Values an update sets before its action's changes run: the next lock version and a
/// new `updated_at`.
pub(crate) fn prepare_update_fields(def: &ResourceDef, existing: &FieldMap, fields: &mut FieldMap) {
    if let Some(version) = def.optimistic_lock_attribute() {
        let current = match existing.get(version) {
            Some(Value::Int(n)) => *n,
            _ => 1,
        };
        fields.insert(version.to_string(), Value::Int(current + 1));
    }
    if let Some((_created_at, updated_at)) = def.timestamps {
        fields.insert(
            updated_at.to_string(),
            Value::String(crate::types::UtcDateTimeUsec::now().as_str().to_string()),
        );
    }
}
