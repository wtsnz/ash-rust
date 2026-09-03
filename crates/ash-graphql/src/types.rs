use ash_core::{AttrType, FieldMap, Value as AshValue};
use async_graphql::dynamic::TypeRef;
use async_graphql::{Name, Value as GqlValue};

/// Converts an Ash [`AttrType`] into an `async_graphql` [`TypeRef`].
pub fn attr_type_to_type_ref(
    resource_name: &str,
    field_name: &str,
    ty: AttrType,
    allow_nil: bool,
) -> TypeRef {
    match ty {
        AttrType::Uuid => {
            if allow_nil {
                TypeRef::named(TypeRef::ID)
            } else {
                TypeRef::named_nn(TypeRef::ID)
            }
        }
        AttrType::String => {
            if allow_nil {
                TypeRef::named(TypeRef::STRING)
            } else {
                TypeRef::named_nn(TypeRef::STRING)
            }
        }
        AttrType::Integer => {
            if allow_nil {
                TypeRef::named(TypeRef::INT)
            } else {
                TypeRef::named_nn(TypeRef::INT)
            }
        }
        AttrType::Boolean => {
            if allow_nil {
                TypeRef::named(TypeRef::BOOLEAN)
            } else {
                TypeRef::named_nn(TypeRef::BOOLEAN)
            }
        }
        AttrType::Atom { .. } => {
            let enum_name = enum_type_name(resource_name, field_name);
            if allow_nil {
                TypeRef::named(enum_name)
            } else {
                TypeRef::named_nn(enum_name)
            }
        }
        AttrType::Map => {
            let json_type = "JSON";
            if allow_nil {
                TypeRef::named(json_type)
            } else {
                TypeRef::named_nn(json_type)
            }
        }
        AttrType::Array => {
            if allow_nil {
                TypeRef::named_list(TypeRef::STRING)
            } else {
                TypeRef::named_nn_list_nn(TypeRef::STRING)
            }
        }
    }
}

/// Generates a standardized Enum type name for an atom field on a resource.
pub fn enum_type_name(resource_name: &str, field_name: &str) -> String {
    let mut capitalized_field = String::new();
    let mut capitalize_next = true;
    for ch in field_name.chars() {
        if ch == '_' {
            capitalize_next = true;
        } else if capitalize_next {
            capitalized_field.extend(ch.to_uppercase());
            capitalize_next = false;
        } else {
            capitalized_field.push(ch);
        }
    }
    format!("{}{}Enum", resource_name, capitalized_field)
}

/// Converts an [`ash_core::Value`] into an `async_graphql` [`GqlValue`].
pub fn ash_value_to_graphql_value(val: &AshValue) -> GqlValue {
    match val {
        AshValue::Null => GqlValue::Null,
        AshValue::Bool(b) => GqlValue::Boolean(*b),
        AshValue::Int(n) => GqlValue::Number((*n).into()),
        AshValue::String(s) => GqlValue::String(s.clone()),
        AshValue::Uuid(u) => GqlValue::String(u.to_string()),
        AshValue::Map(m) => {
            let mut obj = async_graphql::indexmap::IndexMap::new();
            for (k, v) in m.iter() {
                obj.insert(Name::new(k), ash_value_to_graphql_value(v));
            }
            GqlValue::Object(obj)
        }
        AshValue::Array(arr) => {
            let list = arr.iter().map(ash_value_to_graphql_value).collect();
            GqlValue::List(list)
        }
    }
}

/// Converts an [`ash_core::Value`] for a specific [`AttrType`] into an `async_graphql` [`GqlValue`].
pub fn ash_value_to_graphql_value_typed(val: &AshValue, ty: AttrType) -> GqlValue {
    match (val, ty) {
        (AshValue::Null, _) => GqlValue::Null,
        (AshValue::String(s), AttrType::Atom { .. }) => GqlValue::Enum(Name::new(s.to_uppercase())),
        _ => ash_value_to_graphql_value(val),
    }
}

/// Converts an `async_graphql` [`GqlValue`] into an [`ash_core::Value`].
pub fn graphql_value_to_ash_value(val: &GqlValue) -> AshValue {
    match val {
        GqlValue::Null => AshValue::Null,
        GqlValue::Boolean(b) => AshValue::Bool(*b),
        GqlValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                AshValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                AshValue::Int(f.round() as i64)
            } else {
                AshValue::Null
            }
        }
        GqlValue::String(s) => {
            if let Ok(u) = uuid::Uuid::parse_str(s) {
                AshValue::Uuid(u)
            } else {
                AshValue::String(s.clone())
            }
        }
        GqlValue::Enum(name) => AshValue::String(name.to_string()),
        GqlValue::List(list) => {
            let arr = list.iter().map(graphql_value_to_ash_value).collect();
            AshValue::Array(arr)
        }
        GqlValue::Object(obj) => {
            let mut map = FieldMap::new();
            for (k, v) in obj.iter() {
                map.insert(k.to_string(), graphql_value_to_ash_value(v));
            }
            AshValue::Map(map)
        }
        _ => AshValue::Null,
    }
}
