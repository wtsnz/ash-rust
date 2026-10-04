//! What a generic action returns, as a value, for an API that runs it by name: an Ash
//! type's value, else what it serializes to. The macros call it; it's not for code to.

use crate::error::{Error, Result};
use crate::types::AshType;
use crate::value::Value;

/// A generic action's result, waiting to become a value.
pub struct Returned<T>(pub T);

/// An Ash type's own value.
pub trait AsAshValue {
    fn value(&self) -> Result<Value>;
}

impl<T: AshType> AsAshValue for &&Returned<T> {
    fn value(&self) -> Result<Value> {
        Ok(self.0.to_value())
    }
}

/// Anything serializable, as the JSON it serializes to.
pub trait AsSerializedValue {
    fn value(&self) -> Result<Value>;
}

impl<T: serde::Serialize> AsSerializedValue for &Returned<T> {
    fn value(&self) -> Result<Value> {
        serde_json::to_value(&self.0).map(Value::from_plain_json).map_err(|e| Error::Invalid(e.to_string()))
    }
}

/// Anything else, which has no value.
pub trait NoValue {
    fn value(&self) -> Result<Value>;
}

impl<T> NoValue for Returned<T> {
    fn value(&self) -> Result<Value> {
        Err(Error::Invalid(format!(
            "a generic action returning {} can't be run by name: it's neither an Ash type nor serializable",
            std::any::type_name::<T>()
        )))
    }
}
