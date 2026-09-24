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
        None => def.primary_read().ok_or(Error::NoPrimaryRead(def.name))?,
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

pub fn split_input(action: &ActionDef, input: FieldMap) -> Result<(FieldMap, FieldMap)> {
    let mut fields = FieldMap::new();
    let mut arguments = FieldMap::new();
    for (field, value) in input {
        if action.accept.contains(&field.as_str()) {
            fields.insert(field, value);
        } else if action.has_argument(&field) {
            arguments.insert(field, value);
        } else {
            return Err(Error::NotAccepted {
                field,
                action: action.name,
            });
        }
    }
    for arg in action.arguments {
        if !arg.allow_nil && !arguments.contains_key(arg.name) {
            return Err(Error::Missing {
                field: arg.name.to_string(),
            });
        }
    }
    Ok((fields, arguments))
}

#[allow(dead_code)]
pub fn accept(action: &ActionDef, input: FieldMap) -> Result<FieldMap> {
    let (fields, _args) = split_input(action, input)?;
    Ok(fields)
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
    for validation in action.validations {
        match validation {
            Validation::Present { field } => match get_val(field) {
                None | Some(Value::Null) => {
                    return Err(Error::Validation {
                        field: (*field).to_string(),
                        message: "must be present".to_string(),
                    });
                }
                Some(Value::String(s)) if s.trim().is_empty() => {
                    return Err(Error::Validation {
                        field: (*field).to_string(),
                        message: "must be present".to_string(),
                    });
                }
                _ => {}
            },
            Validation::StringLength { field, min, max } => {
                if let Some(Value::String(s)) = get_val(field) {
                    let char_count = s.chars().count();
                    if let Some(min_val) = min
                        && char_count < *min_val
                    {
                        return Err(Error::Validation {
                            field: (*field).to_string(),
                            message: format!("must be at least {min_val} characters"),
                        });
                    }
                    if let Some(max_val) = max
                        && char_count > *max_val
                    {
                        return Err(Error::Validation {
                            field: (*field).to_string(),
                            message: format!("must be at most {max_val} characters"),
                        });
                    }
                }
            }
            Validation::OneOf { field, allowed } => {
                if let Some(Value::String(s)) = get_val(field)
                    && !allowed.contains(&s.as_str())
                {
                    return Err(Error::Validation {
                        field: (*field).to_string(),
                        message: format!("must be one of: {}", allowed.join(", ")),
                    });
                }
            }
            Validation::Numericality { field, min, max } => {
                if let Some(Value::Int(n)) = get_val(field) {
                    if let Some(min_val) = min
                        && *n < *min_val
                    {
                        return Err(Error::Validation {
                            field: (*field).to_string(),
                            message: format!("must be at least {min_val}"),
                        });
                    }
                    if let Some(max_val) = max
                        && *n > *max_val
                    {
                        return Err(Error::Validation {
                            field: (*field).to_string(),
                            message: format!("must be at most {max_val}"),
                        });
                    }
                }
            }
            Validation::Custom(c) => {
                let ctx = crate::action::ValidationContext {
                    record,
                    fields,
                    actor,
                    tenant,
                    metadata,
                    arguments,
                };
                c.validate(&ctx)?;
            }
            Validation::Func(f) => {
                let ctx = crate::action::ValidationContext {
                    record,
                    fields,
                    actor,
                    tenant,
                    metadata,
                    arguments,
                };
                f(&ctx)?;
            }
        }
    }
    Ok(())
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
    let mut filter = filter;
    if let Some(mt) = resource.multitenancy {
        match mt.strategy {
            crate::resource::MultitenancyStrategy::Attribute(attr_name) => {
                if let Some(ref tenant) = tenant {
                    let tenant_filter = Filter::eq(attr_name, Value::String(tenant.clone()));
                    filter = and_filters(filter, Some(tenant_filter));
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
    Ok((filter, tenant))
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
pub fn take_accepted_and_args(action: &ActionDef, input: FieldMap) -> Result<(FieldMap, FieldMap)> {
    let mut fields = FieldMap::new();
    let mut arguments = FieldMap::new();
    for (field, value) in input {
        if action.has_argument(&field) {
            arguments.insert(field, value);
        } else if action.accept.is_empty() || action.accept.contains(&field.as_str()) {
            fields.insert(field, value);
        }
    }
    for arg in action.arguments {
        if !arg.allow_nil && !arguments.contains_key(arg.name) {
            return Err(Error::Missing {
                field: arg.name.to_string(),
            });
        }
    }
    Ok((fields, arguments))
}

pub fn validate(def: &ResourceDef, fields: &FieldMap) -> Result<()> {
    for attribute in def.attributes {
        match fields.get(attribute.name) {
            None | Some(Value::Null) if attribute.allow_nil => {}
            None | Some(Value::Null) => {
                return Err(Error::Missing {
                    field: attribute.name.to_string(),
                });
            }
            Some(value) => check_type(attribute, value)?,
        }
    }
    Ok(())
}

fn check_type(attribute: &AttributeDef, value: &Value) -> Result<()> {
    let ok = match (attribute.ty, value) {
        (AttrType::Uuid, Value::Uuid(_))
        | (AttrType::String, Value::String(_))
        | (AttrType::Integer, Value::Int(_))
        | (AttrType::Boolean, Value::Bool(_))
        | (AttrType::Map, Value::Map(_))
        | (AttrType::Array, Value::Array(_)) => true,
        (AttrType::UtcDatetime, Value::String(got)) => match crate::UtcDateTime::parse(got) {
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
        (AttrType::Date, Value::String(got)) => match crate::Date::parse(got) {
            Ok(_) => true,
            Err(_) => {
                return Err(Error::Constraint {
                    field: attribute.name.to_string(),
                    message: format!("must be a calendar date, got {got}"),
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
        (AttrType::Atom { one_of }, Value::String(got)) => {
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
