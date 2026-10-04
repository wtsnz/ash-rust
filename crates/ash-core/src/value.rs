use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    /// A number that isn't an integer, as JSON holds it (a float attribute is stored as
    /// its canonical text).
    Float(f64),
    Uuid(Uuid),
    String(String),
    Map(FieldMap),
    Array(Vec<Value>),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Self::Float(n) => Some(*n),
            Self::Int(n) => Some(*n as f64),
            _ => None,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_uuid(&self) -> Option<Uuid> {
        match self {
            Self::Uuid(u) => Some(*u),
            _ => None,
        }
    }

    pub fn as_map(&self) -> Option<&FieldMap> {
        match self {
            Self::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }

    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string(self).map_err(|e| Error::Invalid(e.to_string()))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        serde_json::from_str(s).map_err(|e| Error::Invalid(e.to_string()))
    }

    /// Plain JSON for a JSON column: maps become objects and strings stay strings, unlike
    /// [`Value::to_json`], which tags each value with its variant.
    pub fn to_plain_json(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(b) => serde_json::Value::Bool(*b),
            Self::Int(i) => serde_json::Value::from(*i),
            Self::Float(n) => serde_json::Number::from_f64(*n).map_or(serde_json::Value::Null, serde_json::Value::Number),
            Self::Uuid(u) => serde_json::Value::String(u.to_string()),
            Self::String(s) => serde_json::Value::String(s.clone()),
            Self::Map(m) => serde_json::Value::Object(
                m.iter().map(|(k, v)| (k.clone(), v.to_plain_json())).collect(),
            ),
            Self::Array(a) => serde_json::Value::Array(a.iter().map(Self::to_plain_json).collect()),
        }
    }

    /// Reads plain JSON from a JSON column. Integers become [`Value::Int`], other numbers
    /// [`Value::Float`], and strings that parse as UUIDs [`Value::Uuid`].
    pub fn from_plain_json(json: serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Self::Null,
            serde_json::Value::Bool(b) => Self::Bool(b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Self::Int(i),
                None => n.as_f64().map_or_else(|| Self::String(n.to_string()), Self::Float),
            },
            serde_json::Value::String(s) => match Uuid::parse_str(&s) {
                Ok(u) => Self::Uuid(u),
                Err(_) => Self::String(s),
            },
            serde_json::Value::Array(a) => {
                Self::Array(a.into_iter().map(Self::from_plain_json).collect())
            }
            serde_json::Value::Object(o) => Self::Map(
                o.into_iter()
                    .map(|(k, v)| (k, Self::from_plain_json(v)))
                    .collect(),
            ),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
            Self::Float(_) => "float",
            Self::Uuid(_) => "uuid",
            Self::String(_) => "string",
            Self::Map(_) => "map",
            Self::Array(_) => "array",
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "nil"),
            Self::Bool(v) => write!(f, "{v}"),
            Self::Int(v) => write!(f, "{v}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::Uuid(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::Map(m) => write!(f, "{}", serde_json::to_string(m).unwrap_or_default()),
            Self::Array(a) => write!(f, "{}", serde_json::to_string(a).unwrap_or_default()),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Value {}

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        fn rank(value: &Value) -> u8 {
            match value {
                Value::Null => 0,
                Value::Bool(_) => 1,
                Value::Int(_) | Value::Float(_) => 2,
                Value::Uuid(_) => 3,
                Value::String(_) => 4,
                Value::Map(_) => 5,
                Value::Array(_) => 6,
            }
        }

        match (self, other) {
            (Self::Null, Self::Null) => Ordering::Equal,
            (Self::Bool(a), Self::Bool(b)) => a.cmp(b),
            (Self::Int(a), Self::Int(b)) => a.cmp(b),
            (Self::Float(a), Self::Float(b)) => a.total_cmp(b),
            // Numbers by value; an integer and a float of the same value, the integer first.
            (Self::Int(a), Self::Float(b)) => (*a as f64).total_cmp(b).then(Ordering::Less),
            (Self::Float(a), Self::Int(b)) => a.total_cmp(&(*b as f64)).then(Ordering::Greater),
            (Self::Uuid(a), Self::Uuid(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::Array(a), Self::Array(b)) => a.cmp(b),
            (Self::Map(a), Self::Map(b)) => {
                if a.len() != b.len() {
                    return a.len().cmp(&b.len());
                }
                let mut a_entries: Vec<_> = a.iter().collect();
                let mut b_entries: Vec<_> = b.iter().collect();
                a_entries.sort_by_key(|(k, _)| *k);
                b_entries.sort_by_key(|(k, _)| *k);
                a_entries.cmp(&b_entries)
            }
            (a, b) => rank(a).cmp(&rank(b)),
        }
    }
}

impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            Self::Null => {}
            Self::Bool(b) => b.hash(state),
            Self::Int(n) => n.hash(state),
            Self::Float(n) => n.to_bits().hash(state),
            Self::Uuid(u) => u.hash(state),
            Self::String(s) => s.hash(state),
            Self::Array(items) => items.hash(state),
            // Maps compare equal whatever their iteration order, so hash them sorted.
            Self::Map(map) => {
                let mut entries: Vec<_> = map.iter().collect();
                entries.sort_by_key(|(k, _)| *k);
                entries.hash(state);
            }
        }
    }
}

impl From<FieldMap> for Value {
    fn from(value: FieldMap) -> Self {
        Self::Map(value)
    }
}

/// A list holds its items as their type stores them.
impl<T: crate::AshType> From<Vec<T>> for Value {
    fn from(value: Vec<T>) -> Self {
        crate::AshType::to_value(&value)
    }
}

impl From<Vec<Value>> for Value {
    fn from(value: Vec<Value>) -> Self {
        Self::Array(value)
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Self::String(value.to_string())
    }
}

impl From<Uuid> for Value {
    fn from(value: Uuid) -> Self {
        Self::Uuid(value)
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

macro_rules! impl_from_int {
    ($($t:ty),*) => {
        $(
            impl From<$t> for Value {
                fn from(value: $t) -> Self {
                    Self::Int(value as i64)
                }
            }
        )*
    };
}

impl_from_int!(i8, i16, i32, isize, u8, u16, u32, u64, usize, i128, u128);

impl From<Option<Uuid>> for Value {
    fn from(value: Option<Uuid>) -> Self {
        match value {
            Some(id) => Self::Uuid(id),
            None => Self::Null,
        }
    }
}

impl From<Option<i64>> for Value {
    fn from(value: Option<i64>) -> Self {
        match value {
            Some(v) => Self::Int(v),
            None => Self::Null,
        }
    }
}

macro_rules! impl_from_option_int {
    ($($t:ty),*) => {
        $(
            impl From<Option<$t>> for Value {
                fn from(value: Option<$t>) -> Self {
                    match value {
                        Some(v) => Self::Int(v as i64),
                        None => Self::Null,
                    }
                }
            }
        )*
    };
}

impl_from_option_int!(i8, i16, i32, isize, u8, u16, u32, u64, usize, i128, u128);

impl From<Option<bool>> for Value {
    fn from(value: Option<bool>) -> Self {
        match value {
            Some(v) => Self::Bool(v),
            None => Self::Null,
        }
    }
}

impl From<Option<String>> for Value {
    fn from(value: Option<String>) -> Self {
        match value {
            Some(v) => Self::String(v),
            None => Self::Null,
        }
    }
}

/// Ergonomic conversion trait into `Option<T>`.
///
/// Allows action setters for optional fields to accept:
/// - Plain values: `.description("A task")`
/// - String slices: `.description("A task")`
/// - `Some(...)`: `.description(Some("A task"))`
/// - `None`: `.description(None)`
pub trait IntoOption<T> {
    fn into_option(self) -> Option<T>;
}

impl<T> IntoOption<T> for Option<T> {
    fn into_option(self) -> Option<T> {
        self
    }
}

/// Any value is a present value of its own type, so a setter for an optional field of
/// a new type works without an impl of its own.
impl<T> IntoOption<T> for T {
    fn into_option(self) -> Option<T> {
        Some(self)
    }
}

impl IntoOption<String> for &str {
    fn into_option(self) -> Option<String> {
        Some(self.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConstValue {
    Null,
    Bool(bool),
    Int(i64),
    Str(&'static str),
}

impl From<ConstValue> for Value {
    fn from(value: ConstValue) -> Self {
        match value {
            ConstValue::Null => Self::Null,
            ConstValue::Bool(v) => Self::Bool(v),
            ConstValue::Int(v) => Self::Int(v),
            ConstValue::Str(v) => Self::String(v.to_string()),
        }
    }
}

pub type FieldMap = HashMap<String, Value>;

pub fn required_uuid(fields: &FieldMap, key: &str) -> Result<Uuid> {
    match fields.get(key) {
        Some(Value::Uuid(id)) => Ok(*id),
        Some(value) => Err(Error::TypeMismatch {
            field: key.to_string(),
            expected: "uuid".into(),
            got: value.type_name().into(),
        }),
        None => Err(Error::Missing {
            field: key.to_string(),
        }),
    }
}

pub fn required_string(fields: &FieldMap, key: &str) -> Result<String> {
    match fields.get(key) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(value) => Err(Error::TypeMismatch {
            field: key.to_string(),
            expected: "string".into(),
            got: value.type_name().into(),
        }),
        None => Err(Error::Missing {
            field: key.to_string(),
        }),
    }
}

pub fn optional_int(fields: &FieldMap, key: &str) -> Result<Option<i64>> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Int(value)) => Ok(Some(*value)),
        Some(value) => Err(Error::TypeMismatch {
            field: key.to_string(),
            expected: "integer".into(),
            got: value.type_name().into(),
        }),
    }
}

pub fn optional_uuid(fields: &FieldMap, key: &str) -> Result<Option<Uuid>> {
    match fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Uuid(id)) => Ok(Some(*id)),
        Some(value) => Err(Error::TypeMismatch {
            field: key.to_string(),
            expected: "uuid".into(),
            got: value.type_name().into(),
        }),
    }
}

/// Where a record notes its metadata `name`, among its fields.
fn metadata_key(name: &str) -> String {
    format!("__metadata__:{name}")
}

/// Notes `value` as `record`'s metadata `name`, as Ash's `Ash.Resource.put_metadata/3`:
/// something the action that answers the record says of it, beyond its fields (declared
/// on the action as [`crate::MetadataDef`]s for an API to show).
pub fn put_metadata(record: &mut FieldMap, name: &str, value: impl Into<Value>) {
    record.insert(metadata_key(name), value.into());
}

/// `record`'s metadata `name`, as Ash's `Ash.Resource.get_metadata/2`.
pub fn get_metadata<'a>(record: &'a FieldMap, name: &str) -> Option<&'a Value> {
    record.get(&metadata_key(name))
}

/// A union's value: its member `name` holding `value`, as Ash holds a union
/// (`{type, value}`).
pub fn union_value(name: &str, value: Value) -> Value {
    let mut held = FieldMap::new();
    held.insert("type".to_string(), Value::String(name.to_string()));
    held.insert("value".to_string(), value);
    Value::Map(held)
}

impl Value {
    /// The member a union's value holds, and its value.
    pub fn union_member(&self) -> Option<(&str, &Value)> {
        match self {
            Value::Map(held) => match (held.get("type"), held.get("value")) {
                (Some(Value::String(name)), Some(value)) => Some((name.as_str(), value)),
                _ => None,
            },
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, RandomState};

    #[test]
    fn equal_maps_hash_alike_whatever_their_order() {
        let mut forward = FieldMap::new();
        let mut backward = FieldMap::new();
        for n in 0..32 {
            forward.insert(format!("k{n}"), Value::Int(n));
        }
        for n in (0..32).rev() {
            backward.insert(format!("k{n}"), Value::Int(n));
        }
        let (forward, backward) = (Value::Map(forward), Value::Map(backward));
        assert_eq!(forward, backward);
        let hasher = RandomState::new();
        assert_eq!(hasher.hash_one(&forward), hasher.hash_one(&backward));
        assert_ne!(
            hasher.hash_one(Value::Int(1)),
            hasher.hash_one(Value::Bool(true))
        );
    }
}
