use std::collections::HashMap;

use uuid::Uuid;

use crate::value::{ConstValue, Value};

#[derive(Clone, Debug)]
pub struct Actor {
    pub id: Uuid,
    attributes: HashMap<String, Value>,
}

impl Actor {
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            attributes: HashMap::new(),
        }
    }

    pub fn with(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    pub fn with_attr(self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.with(key, value)
    }

    pub fn with_role(self, role: impl Into<String>) -> Self {
        self.with("role", Value::String(role.into()))
    }

    pub fn role(&self) -> Option<&str> {
        match self.attributes.get("role") {
            Some(Value::String(s)) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn attr(&self, key: &str) -> Option<&Value> {
        self.attributes.get(key)
    }

    pub fn attr_eq(&self, key: &str, expected: ConstValue) -> bool {
        match (self.attributes.get(key), expected) {
            (Some(Value::String(value)), ConstValue::Str(expected)) => value == expected,
            (Some(Value::Bool(value)), ConstValue::Bool(expected)) => *value == expected,
            (Some(Value::Int(value)), ConstValue::Int(expected)) => *value == expected,
            (Some(Value::Null), ConstValue::Null) => true,
            _ => false,
        }
    }
}
