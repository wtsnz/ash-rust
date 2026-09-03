use uuid::Uuid;

use crate::action::{ActionDef, ActionKind, Change, PersistKind, Validation};
use crate::actor::Actor;
use crate::error::{Error, Result};
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
    for change in action.changes {
        match change {
            Change::SetAttribute { field, value } => {
                fields.insert((*field).to_string(), Value::from(*value));
            }
            Change::SetNewAttribute { field, value } => {
                match fields.get(*field) {
                    None | Some(Value::Null) => {
                        fields.insert((*field).to_string(), Value::from(*value));
                    }
                    _ => {}
                }
            }
            Change::RelateActor { field } => {
                let actor = actor.ok_or(Error::Forbidden)?;
                fields.insert((*field).to_string(), Value::Uuid(actor.id));
            }
            Change::SetFromArgument { field, argument } => {
                if let Some(val) = arguments.get(*argument) {
                    fields.insert((*field).to_string(), val.clone());
                }
            }
            Change::Custom(c) => {
                let mut ctx = crate::action::ChangeContext {
                    fields,
                    actor,
                    arguments,
                };
                c.apply(&mut ctx)?;
            }
            Change::ManageRelationship { .. } => {}
            Change::Func(f) => {
                let mut ctx = crate::action::ChangeContext {
                    fields,
                    actor,
                    arguments,
                };
                f(&mut ctx)?;
            }
        }
    }
    Ok(())
}

pub fn run_validations(
    _def: &ResourceDef,
    action: &ActionDef,
    record: Option<&FieldMap>,
    fields: &FieldMap,
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
                    arguments,
                };
                c.validate(&ctx)?;
            }
            Validation::Func(f) => {
                let ctx = crate::action::ValidationContext {
                    record,
                    fields,
                    arguments,
                };
                f(&ctx)?;
            }
        }
    }
    Ok(())
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
