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
        AttrType::String
        | AttrType::CiString
        | AttrType::Date
        | AttrType::Binary
        | AttrType::UtcDatetime
        | AttrType::Inet
        | AttrType::Decimal => {
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
        AttrType::Float => {
            if allow_nil {
                TypeRef::named(TypeRef::FLOAT)
            } else {
                TypeRef::named_nn(TypeRef::FLOAT)
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
        AttrType::Vector { .. } => {
            if allow_nil {
                TypeRef::named_nn_list(TypeRef::FLOAT)
            } else {
                TypeRef::named_nn_list_nn(TypeRef::FLOAT)
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
        (AshValue::String(s), AttrType::Float) => match s.parse::<f64>() {
            Ok(value) => float_value(value),
            Err(_) => GqlValue::Null,
        },
        (AshValue::String(s), AttrType::Vector { .. }) => match ash_core::parse_vector(s) {
            Ok(values) => GqlValue::List(
                values
                    .into_iter()
                    // Widen through the shortest text so 0.1f32 reads as 0.1, not 0.10000000149.
                    .map(|value| float_value(value.to_string().parse().unwrap_or(value as f64)))
                    .collect(),
            ),
            Err(_) => GqlValue::Null,
        },
        _ => ash_value_to_graphql_value(val),
    }
}

fn float_value(value: f64) -> GqlValue {
    async_graphql::Number::from_f64(value)
        .map(GqlValue::Number)
        .unwrap_or(GqlValue::Null)
}

/// Parses an IP address input into its normalized text form.
pub fn parse_inet_input(
    acc: &async_graphql::dynamic::ValueAccessor<'_>,
) -> Result<String, async_graphql::Error> {
    let inet = ash_core::Inet::parse(acc.string()?)
        .map_err(|err| async_graphql::Error::new(err.to_string()))?;
    Ok(inet.as_str().to_string())
}

/// Parses a `[Float!]` input into pgvector text, checking its dimensions.
pub fn parse_vector_input(
    acc: &async_graphql::dynamic::ValueAccessor<'_>,
    dimensions: u32,
) -> Result<String, async_graphql::Error> {
    let mut values = Vec::new();
    for item in acc.list()?.iter() {
        values.push(item.f64()? as f32);
    }
    ash_core::check_vector(&values, dimensions)
        .map_err(|err| async_graphql::Error::new(err.to_string()))?;
    Ok(ash_core::format_vector(&values))
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

/// Parses an `async_graphql` [`ValueAccessor`] into an [`ash_core::Value`] based on [`AttrType`].
pub fn parse_input_val(
    acc: &async_graphql::dynamic::ValueAccessor<'_>,
    ty: AttrType,
) -> Result<AshValue, async_graphql::Error> {
    match ty {
        AttrType::Uuid => {
            let s = acc.string()?;
            let u = uuid::Uuid::parse_str(s)
                .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;
            Ok(AshValue::Uuid(u))
        }
        AttrType::String => {
            let s = acc.string()?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::UtcDatetime => {
            let s = acc.string()?;
            ash_core::UtcDateTime::parse(s)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::Binary => {
            let s = acc.string()?;
            let binary =
                ash_core::Binary::parse(s).map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(binary.encode()))
        }
        AttrType::Date => {
            let s = acc.string()?;
            ash_core::Date::parse(s).map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::CiString => {
            let s = acc.string()?;
            let value = ash_core::CiString::parse(s)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(value.as_str().to_string()))
        }
        AttrType::Decimal => {
            let s = acc.string()?;
            ash_core::Decimal::parse(s)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::Float => {
            let n = acc.f64()?;
            let float = ash_core::Float::parse(&n.to_string())
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(float.as_str().to_string()))
        }
        AttrType::Integer => {
            let n = acc.i64()?;
            Ok(AshValue::Int(n))
        }
        AttrType::Boolean => {
            let b = acc.boolean()?;
            Ok(AshValue::Bool(b))
        }
        AttrType::Inet => Ok(AshValue::String(parse_inet_input(acc)?)),
        AttrType::Vector { dimensions } => {
            Ok(AshValue::String(parse_vector_input(acc, dimensions)?))
        }
        AttrType::Atom { one_of } => {
            let name = acc.enum_name()?;
            if let Some(matched) = one_of.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
                Ok(AshValue::String((*matched).to_string()))
            } else {
                Ok(AshValue::String(name.to_string()))
            }
        }
        _ => Ok(AshValue::Null),
    }
}
