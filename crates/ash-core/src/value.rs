use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{Error, Result};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
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

    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Int(_) => "integer",
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
            Self::Uuid(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::Map(m) => write!(f, "{}", serde_json::to_string(m).unwrap_or_default()),
            Self::Array(a) => write!(f, "{}", serde_json::to_string(a).unwrap_or_default()),
        }
    }
}

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
                Value::Int(_) => 2,
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
            (Self::Uuid(a), Self::Uuid(b)) => a.cmp(b),
            (Self::String(a), Self::String(b)) => a.cmp(b),
            (Self::Array(a), Self::Array(b)) => a.cmp(b),
            (Self::Map(a), Self::Map(b)) => {
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

impl From<FieldMap> for Value {
    fn from(value: FieldMap) -> Self {
        Self::Map(value)
    }
}

impl From<Vec<FieldMap>> for Value {
    fn from(value: Vec<FieldMap>) -> Self {
        Self::Array(value.into_iter().map(Value::Map).collect())
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

impl IntoOption<String> for String {
    fn into_option(self) -> Option<String> {
        Some(self)
    }
}

impl IntoOption<String> for &str {
    fn into_option(self) -> Option<String> {
        Some(self.to_string())
    }
}

impl IntoOption<i64> for i64 {
    fn into_option(self) -> Option<i64> {
        Some(self)
    }
}

macro_rules! impl_into_option_int {
    ($($t:ty),*) => {
        $(
            impl IntoOption<$t> for $t {
                fn into_option(self) -> Option<$t> {
                    Some(self)
                }
            }
        )*
    };
}

impl_into_option_int!(i8, i16, i32, isize, u8, u16, u32, u64, usize, i128, u128);

impl IntoOption<bool> for bool {
    fn into_option(self) -> Option<bool> {
        Some(self)
    }
}

impl IntoOption<Uuid> for Uuid {
    fn into_option(self) -> Option<Uuid> {
        Some(self)
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
