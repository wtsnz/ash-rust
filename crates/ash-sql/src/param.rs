use ash_core::{AttrType, Value};

/// Represents a bound SQL parameter value.
#[derive(Clone, Debug, PartialEq)]
pub struct SqlParam {
    pub value: Value,
    pub is_list: bool,
    /// The value is base64 and the driver should bind decoded bytes.
    pub binary: bool,
    /// The column type, when known, so a driver can bind NULL with the right type.
    pub ty: Option<AttrType>,
}

impl SqlParam {
    pub fn new(value: Value) -> Self {
        Self {
            value,
            is_list: false,
            binary: false,
            ty: None,
        }
    }

    pub fn binary(value: Value) -> Self {
        Self {
            value,
            is_list: false,
            binary: true,
            ty: Some(AttrType::Binary),
        }
    }

    pub fn list(values: Vec<Value>) -> Self {
        Self {
            value: Value::Array(values),
            is_list: true,
            binary: false,
            ty: None,
        }
    }

    pub fn typed(value: Value, ty: AttrType) -> Self {
        Self {
            ty: Some(ty),
            ..Self::new(value)
        }
    }
}

impl From<Value> for SqlParam {
    fn from(value: Value) -> Self {
        Self::new(value)
    }
}

impl From<&Value> for SqlParam {
    fn from(value: &Value) -> Self {
        Self::new(value.clone())
    }
}

/// Helper converting a slice of [`Value`]s into a normalized JSON array string for `json_each` bindings.
pub fn values_to_json_array(items: &[Value]) -> String {
    let json_items: Vec<serde_json::Value> = items
        .iter()
        .map(|v| match v {
            Value::Null => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Int(i) => serde_json::Value::Number((*i).into()),
            Value::String(s) => serde_json::Value::String(s.clone()),
            Value::Uuid(u) => serde_json::Value::String(u.to_string()),
            Value::Map(m) => serde_json::to_value(m).unwrap_or(serde_json::Value::Null),
            Value::Array(a) => serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
        })
        .collect();
    serde_json::to_string(&json_items).unwrap_or_else(|_| "[]".to_string())
}
