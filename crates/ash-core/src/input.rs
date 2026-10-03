//! A client's filter, sort and values, as JSON: Ash's `filter_input` and `sort_input`. An
//! API layer (AshTypescript's RPC, say) hands these what its client sent, with field and
//! operator names as Ash spells them (`greater_than`, `comment_count`), and gets the
//! filter, sort and values a read or write takes.
//!
//! - A filter is an object: each field (attribute, aggregate or calculation) names its
//!   operators (`{"priority": {"greater_than_or_equal": 3}}`), a relationship nests the
//!   related resource's filter, and `and`, `or` and `not` take lists of filters (`not`
//!   excluding records matching all of its).
//! - A sort is text: fields separated by commas, each `field` or `+field` ascending,
//!   `-field` descending, `++field` ascending with nulls first and `--field` descending
//!   with nulls last, as Ash takes them.
//! - A value is cast to its field's type: a UUID from its text, an integer from a number,
//!   text as text (validation canonicalizes it as it writes).

use serde_json::Value as Json;

use crate::data_layer::Sort;
use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::{AttrType, ResourceDef};
use crate::value::{FieldMap, Value};

fn invalid(message: impl Into<String>) -> Error {
    Error::Invalid(message.into())
}

/// The type of `field` on `resource`: an attribute, aggregate or calculation.
pub fn field_type(resource: &ResourceDef, field: &str) -> Option<AttrType> {
    resource
        .attribute(field)
        .map(|attr| attr.ty)
        .or_else(|| resource.aggregate(field).map(|agg| agg.ty))
        .or_else(|| resource.calculation(field).map(|calc| calc.ty))
}

/// `json` as a value of type `ty`.
pub fn value_input(ty: AttrType, json: &Json) -> Result<Value> {
    let mismatch = || invalid(format!("expected {}, got {json}", ty.name()));
    Ok(match (ty, json) {
        (_, Json::Null) => Value::Null,
        (AttrType::Uuid, Json::String(text)) => {
            Value::Uuid(uuid::Uuid::parse_str(text).map_err(|_| invalid(format!("invalid UUID {text:?}")))?)
        }
        (AttrType::Integer, Json::Number(n)) => Value::Int(n.as_i64().ok_or_else(mismatch)?),
        (AttrType::Integer, Json::String(text)) => Value::Int(text.parse().map_err(|_| mismatch())?),
        (AttrType::Boolean, Json::Bool(b)) => Value::Bool(*b),
        // As Ash's boolean casts text.
        (AttrType::Boolean, Json::String(text)) => match text.as_str() {
            "true" | "1" => Value::Bool(true),
            "false" | "0" => Value::Bool(false),
            _ => return Err(mismatch()),
        },
        (AttrType::Float | AttrType::Decimal, Json::Number(n)) => Value::String(n.to_string()),
        (AttrType::Map, json @ Json::Object(_)) | (AttrType::Array, json @ Json::Array(_)) => Value::from_plain_json(json.clone()),
        (AttrType::Atom { one_of, .. }, Json::String(text)) => Value::String(
            one_of
                .iter()
                .find(|&&known| known.eq_ignore_ascii_case(text))
                .map_or_else(|| text.clone(), |known| known.to_string()),
        ),
        (
            AttrType::String
            | AttrType::CiString
            | AttrType::UtcDatetime { .. }
            | AttrType::Date
            | AttrType::Decimal
            | AttrType::Float
            | AttrType::Binary
            | AttrType::Inet
            | AttrType::Vector { .. },
            Json::String(text),
        ) => Value::String(text.clone()),
        _ => return Err(mismatch()),
    })
}

/// A client's filter on `resource`.
pub fn filter_input(resource: &'static ResourceDef, json: &Json) -> Result<Filter> {
    let Json::Object(fields) = json else {
        return Err(invalid(format!("a filter on {} must be an object", resource.name)));
    };
    let mut filters = Vec::new();
    for (key, value) in fields {
        if value.is_null() {
            continue;
        }
        let list = |value: &Json| -> Result<Vec<Filter>> {
            match value {
                Json::Array(items) => items.iter().map(|item| filter_input(resource, item)).collect(),
                single => Ok(vec![filter_input(resource, single)?]),
            }
        };
        match key.as_str() {
            "and" => filters.push(Filter::and(list(value)?)),
            "or" => filters.push(Filter::or(list(value)?)),
            // `not: [a, b]` excludes the records matching all of them.
            "not" => filters.push(!Filter::and(list(value)?)),
            name => {
                if let Some(ty) = field_type(resource, name) {
                    filters.extend(field_filter(resource, name, ty, value)?);
                } else if let Some(rel) = resource.relationship(name) {
                    let inner = filter_input((rel.destination)(), value)?;
                    if inner != Filter::True {
                        filters.push(Filter::related(rel.name, inner));
                    }
                } else {
                    return Err(invalid(format!("no field `{name}` to filter on {}", resource.name)));
                }
            }
        }
    }
    Ok(Filter::and(filters))
}

fn field_filter(resource: &ResourceDef, field: &str, ty: AttrType, ops: &Json) -> Result<Vec<Filter>> {
    let Json::Object(ops) = ops else {
        // A bare value means equality, as Ash reads `%{field: value}`.
        return Ok(vec![Filter::eq(field, value_input(ty, ops)?)]);
    };
    let text = |value: &Json| -> Result<String> {
        value.as_str().map(str::to_string).ok_or_else(|| invalid(format!("`{field}` takes text here")))
    };
    let mut filters = Vec::new();
    for (op, value) in ops {
        if value.is_null() && !matches!(op.as_str(), "eq" | "is_distinct_from" | "is_not_distinct_from") {
            continue;
        }
        filters.push(match op.as_str() {
            "is_nil" => match value {
                Json::Bool(true) => Filter::is_nil(field),
                Json::Bool(false) => !Filter::is_nil(field),
                _ => return Err(invalid(format!("`is_nil` on `{field}` takes a boolean"))),
            },
            "eq" => Filter::eq(field, value_input(ty, value)?),
            "not_eq" => Filter::ne(field, value_input(ty, value)?),
            "less_than" => Filter::lt(field, value_input(ty, value)?),
            "greater_than" => Filter::gt(field, value_input(ty, value)?),
            "less_than_or_equal" => Filter::lte(field, value_input(ty, value)?),
            "greater_than_or_equal" => Filter::gte(field, value_input(ty, value)?),
            "in" => {
                let items = value.as_array().ok_or_else(|| invalid(format!("`in` on `{field}` takes a list")))?;
                Filter::in_list(field, items.iter().map(|item| value_input(ty, item)).collect::<Result<Vec<_>>>()?)
            }
            // Null-safe equality: nulls are equal to each other and to nothing else.
            "is_distinct_from" => match value_input(ty, value)? {
                Value::Null => !Filter::is_nil(field),
                v => Filter::or([Filter::is_nil(field), Filter::ne(field, v)]),
            },
            "is_not_distinct_from" => match value_input(ty, value)? {
                Value::Null => Filter::is_nil(field),
                v => Filter::eq(field, v),
            },
            "contains" => Filter::contains(field, text(value)?),
            "string_starts_with" => Filter::starts_with(field, text(value)?),
            "string_ends_with" => Filter::ends_with(field, text(value)?),
            "like" => Filter::like(field, text(value)?),
            "ilike" => Filter::ilike(field, text(value)?),
            other => return Err(invalid(format!("unknown operator `{other}` on {}.{field}", resource.name))),
        });
    }
    Ok(filters)
}

/// A client's sort on `resource`: `"-inserted_at,id"`.
pub fn sort_input(resource: &ResourceDef, text: &str) -> Result<Vec<Sort>> {
    text.split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            // `++` and `--` place nulls against the direction's default, as Ash's
            // `asc_nils_first` and `desc_nils_last`.
            let (descending, nulls_first, field) = match part {
                p if p.starts_with("--") => (true, Some(false), &p[2..]),
                p if p.starts_with("++") => (false, Some(true), &p[2..]),
                p if p.starts_with('-') => (true, None, &p[1..]),
                p if p.starts_with('+') => (false, None, &p[1..]),
                p => (false, None, p),
            };
            if field_type(resource, field).is_none() {
                return Err(invalid(format!("no field `{field}` to sort {} by", resource.name)));
            }
            Ok(Sort {
                field: field.to_string(),
                descending,
                nulls_first,
                ..Default::default()
            })
        })
        .collect()
}

/// A client's input to `action` on `resource`, as the action takes it: each accepted
/// attribute and argument, cast to its type. Anything else is refused, as Ash refuses
/// input an action doesn't take, and so is input lacking an argument that may not be nil.
pub fn action_input(resource: &ResourceDef, action: &crate::action::ActionDef, json: &Json) -> Result<FieldMap> {
    let mut input = FieldMap::new();
    let Json::Object(given) = json else {
        return if json.is_null() { Ok(input) } else { Err(invalid("input must be an object")) };
    };
    for (name, value) in given {
        let ty = action
            .arguments
            .iter()
            .find(|arg| arg.name == name)
            .map(|arg| arg.ty)
            .or_else(|| action.accept.contains(&name.as_str()).then(|| resource.attribute(name).map(|attr| attr.ty)).flatten())
            .ok_or_else(|| Error::NotAccepted {
                field: name.clone(),
                action: action.name,
            })?;
        let value = value_input(ty, value).map_err(|_| Error::TypeMismatch {
            field: name.clone(),
            expected: ty.name().to_string(),
            got: value.to_string(),
        })?;
        input.insert(name.clone(), value);
    }
    // An argument that may not be nil must be given, as Ash requires it.
    if let Some(arg) = action.arguments.iter().find(|arg| !arg.allow_nil && input.get(arg.name).is_none_or(Value::is_null)) {
        return Err(Error::Missing { field: arg.name.to_string() });
    }
    Ok(input)
}
