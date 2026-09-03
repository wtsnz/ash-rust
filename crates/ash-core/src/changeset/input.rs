use crate::value::{FieldMap, Value};

/// Converts a type into a [`FieldMap`] for relationship mutations or nested inputs.
pub trait IntoFieldMap {
    fn into_field_map(self) -> FieldMap;
}

impl IntoFieldMap for FieldMap {
    fn into_field_map(self) -> FieldMap {
        self
    }
}

impl IntoFieldMap for &FieldMap {
    fn into_field_map(self) -> FieldMap {
        self.clone()
    }
}

impl IntoFieldMap for Value {
    fn into_field_map(self) -> FieldMap {
        match self {
            Value::Map(m) => m,
            _ => FieldMap::new(),
        }
    }
}

impl<K, V> IntoFieldMap for Vec<(K, V)>
where
    K: Into<String>,
    V: Into<Value>,
{
    fn into_field_map(self) -> FieldMap {
        self.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }
}

impl<K, V, const N: usize> IntoFieldMap for [(K, V); N]
where
    K: Into<String> + Clone,
    V: Into<Value> + Clone,
{
    fn into_field_map(self) -> FieldMap {
        self.into_iter().map(|(k, v)| (k.into(), v.into())).collect()
    }
}
