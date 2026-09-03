use ash_core::Value;

/// Represents a bound SQL parameter value.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlParam {
    pub value: Value,
}

impl SqlParam {
    pub fn new(value: Value) -> Self {
        Self { value }
    }
}

impl From<Value> for SqlParam {
    fn from(value: Value) -> Self {
        Self { value }
    }
}

impl From<&Value> for SqlParam {
    fn from(value: &Value) -> Self {
        Self {
            value: value.clone(),
        }
    }
}
