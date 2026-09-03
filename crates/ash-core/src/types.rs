use crate::error::{Error, Result};
use crate::resource::AttrType;
use crate::value::Value;
use uuid::Uuid;

/// Core trait for types usable as attribute fields in Ash resources.
pub trait AshType: Sized + Clone + Send + Sync + 'static {
    const ATTR_TYPE: AttrType;
    fn to_value(&self) -> Value;
    fn from_value(value: &Value) -> Result<Self>;
}

/// Trait implemented by native Rust enums used as Ash atom/enum types.
pub trait AshEnum: AshType {
    const VARIANTS: &'static [&'static str];
    fn as_str(&self) -> &'static str;
    fn parse(s: &str) -> Result<Self>;
}

impl AshType for Uuid {
    const ATTR_TYPE: AttrType = AttrType::Uuid;

    fn to_value(&self) -> Value {
        Value::Uuid(*self)
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Uuid(u) => Ok(*u),
            Value::String(s) => Uuid::parse_str(s).map_err(|e| Error::Invalid(e.to_string())),
            _ => Err(Error::Invalid("expected uuid".into())),
        }
    }
}

impl AshType for String {
    const ATTR_TYPE: AttrType = AttrType::String;

    fn to_value(&self) -> Value {
        Value::String(self.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Ok(s.clone()),
            _ => Err(Error::Invalid("expected string".into())),
        }
    }
}

impl AshType for i64 {
    const ATTR_TYPE: AttrType = AttrType::Integer;

    fn to_value(&self) -> Value {
        Value::Int(*self)
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Int(n) => Ok(*n),
            _ => Err(Error::Invalid("expected integer".into())),
        }
    }
}

impl AshType for bool {
    const ATTR_TYPE: AttrType = AttrType::Boolean;

    fn to_value(&self) -> Value {
        Value::Bool(*self)
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Bool(b) => Ok(*b),
            _ => Err(Error::Invalid("expected boolean".into())),
        }
    }
}

impl<T: AshType> AshType for Option<T> {
    const ATTR_TYPE: AttrType = T::ATTR_TYPE;

    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null,
        }
    }

    fn from_value(value: &Value) -> Result<Self> {
        if value.is_null() {
            Ok(None)
        } else {
            T::from_value(value).map(Some)
        }
    }
}
