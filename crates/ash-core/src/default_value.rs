//! An attribute's default, as a value: the attribute's own type, stored as that type
//! stores it (a list's items included), or text the attribute's type casts
//! (`Decimal = "12.50"`). The macros call it; it's not for code to.

use crate::types::AshType;
use crate::value::Value;

/// A default for an attribute of type `T`.
pub trait IntoDefault<T> {
    fn into_default(self) -> Value;
}

impl<T: AshType> IntoDefault<T> for T {
    fn into_default(self) -> Value {
        self.to_value()
    }
}

impl<T> IntoDefault<T> for &'static str {
    fn into_default(self) -> Value {
        Value::from(self)
    }
}
