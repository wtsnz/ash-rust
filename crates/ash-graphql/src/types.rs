use ash_core::{AttrType, FieldMap, Value as AshValue};
use async_graphql::dynamic::TypeRef;
use async_graphql::{Name, Value as GqlValue};

/// The GraphQL scalar or enum an Ash type is, by name, as AshGraphql maps them: UUIDs are
/// `ID`, UTC datetimes `DateTime`, maps `Json`, and an enum type (a named atom) its own
/// enum. An unnamed atom is constrained text, a `String`, as a plain `:atom` is.
pub fn graphql_type_name(ty: AttrType) -> &'static str {
    match ty {
        AttrType::Uuid => TypeRef::ID,
        AttrType::Integer => TypeRef::INT,
        AttrType::Float => TypeRef::FLOAT,
        AttrType::Boolean => TypeRef::BOOLEAN,
        AttrType::UtcDatetime { .. } => "DateTime",
        AttrType::Date => "Date",
        AttrType::Decimal => "Decimal",
        AttrType::Map => "Json",
        // Their own types, as AshGraphql gives them (see `composite`).
        AttrType::Embedded(embedded) => embedded.resource().name,
        AttrType::TypedMap { name, .. } | AttrType::Union { name, .. } => name,
        AttrType::Atom { name: Some(name), .. } => name,
        AttrType::Atom { name: None, .. }
        | AttrType::String
        | AttrType::CiString
        | AttrType::Binary
        | AttrType::Inet
        | AttrType::Vector { .. } => TypeRef::STRING,
        // A list within a list has no GraphQL type of its own here.
        AttrType::Array { .. } => "Json",
    }
}

/// The custom scalars the schema's types use.
pub const CUSTOM_SCALARS: &[&str] = &["DateTime", "Date", "Decimal", "Json"];

/// The input type a value of `ty` given as `field` of `owner` takes: an embedded
/// resource's input for that field (`<Owner><Field>Input`), a typed map's or union's
/// `<Name>Input`, a list of them, or the type's own.
pub fn input_type_ref(owner: &str, field: &str, ty: AttrType, allow_nil: bool) -> TypeRef {
    let name = |ty: AttrType| match ty {
        AttrType::Embedded(_) => format!("{owner}{}Input", crate::names::pascal(field)),
        AttrType::TypedMap { name, .. } | AttrType::Union { name, .. } => format!("{name}Input"),
        other => graphql_type_name(other).to_string(),
    };
    match ty {
        AttrType::Array { of } if crate::composite::is_composite(*of) => {
            if allow_nil {
                TypeRef::named_nn_list(name(*of))
            } else {
                TypeRef::named_nn_list_nn(name(*of))
            }
        }
        AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => {
            if allow_nil {
                TypeRef::named(name(ty))
            } else {
                TypeRef::named_nn(name(ty))
            }
        }
        _ => attr_type_to_type_ref(owner, field, ty, allow_nil),
    }
}

/// Converts an Ash [`AttrType`] into an `async_graphql` [`TypeRef`].
pub fn attr_type_to_type_ref(
    _resource_name: &str,
    _field_name: &str,
    ty: AttrType,
    allow_nil: bool,
) -> TypeRef {
    match ty {
        // A list of its items' type, as AshGraphql serves `{:array, type}`.
        AttrType::Array { of } => {
            let item = graphql_type_name(*of);
            if allow_nil {
                TypeRef::named_nn_list(item)
            } else {
                TypeRef::named_nn_list_nn(item)
            }
        }
        AttrType::Vector { .. } => {
            if allow_nil {
                TypeRef::named_nn_list(TypeRef::FLOAT)
            } else {
                TypeRef::named_nn_list_nn(TypeRef::FLOAT)
            }
        }
        _ if allow_nil => TypeRef::named(graphql_type_name(ty)),
        _ => TypeRef::named_nn(graphql_type_name(ty)),
    }
}

/// The enum type a named atom is, if it is one.
pub fn enum_type_name(ty: AttrType) -> Option<&'static str> {
    match ty {
        AttrType::Atom { name: Some(name), .. } => Some(name),
        _ => None,
    }
}

/// Converts an [`ash_core::Value`] into an `async_graphql` [`GqlValue`].
pub fn ash_value_to_graphql_value(val: &AshValue) -> GqlValue {
    match val {
        AshValue::Null => GqlValue::Null,
        AshValue::Bool(b) => GqlValue::Boolean(*b),
        AshValue::Int(n) => GqlValue::Number((*n).into()),
        AshValue::Float(n) => async_graphql::Number::from_f64(*n).map_or(GqlValue::Null, GqlValue::Number),
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
        (AshValue::String(s), AttrType::Atom { name: Some(_), .. }) => {
            GqlValue::Enum(Name::new(s.to_uppercase()))
        }
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
    inet_text(acc.as_value())
}

fn inet_text(value: &GqlValue) -> Result<String, async_graphql::Error> {
    let inet = ash_core::Inet::parse(string(value)?)
        .map_err(|err| async_graphql::Error::new(err.to_string()))?;
    Ok(inet.as_str().to_string())
}

/// Parses a `[Float!]` input into pgvector text, checking its dimensions.
pub fn parse_vector_input(
    acc: &async_graphql::dynamic::ValueAccessor<'_>,
    dimensions: u32,
) -> Result<String, async_graphql::Error> {
    vector_text(acc.as_value(), dimensions)
}

fn vector_text(value: &GqlValue, dimensions: u32) -> Result<String, async_graphql::Error> {
    let mut values = Vec::new();
    for item in crate::filter::list(value) {
        values.push(float(item)? as f32);
    }
    ash_core::check_vector(&values, dimensions)
        .map_err(|err| async_graphql::Error::new(err.to_string()))?;
    Ok(ash_core::format_vector(&values))
}

fn string(value: &GqlValue) -> Result<&str, async_graphql::Error> {
    match value {
        GqlValue::String(text) => Ok(text),
        _ => Err(async_graphql::Error::new("internal: not a string")),
    }
}

fn float(value: &GqlValue) -> Result<f64, async_graphql::Error> {
    match value {
        GqlValue::Number(n) => n.as_f64().ok_or_else(|| async_graphql::Error::new("internal: not a float")),
        _ => Err(async_graphql::Error::new("internal: not a float")),
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
                AshValue::Float(f)
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
    parse_input_value(acc.as_value(), ty)
}

/// [`parse_input_val`] of the value as given: an argument's, or one a selection holds.
pub fn parse_input_value(value: &GqlValue, ty: AttrType) -> Result<AshValue, async_graphql::Error> {
    match ty {
        AttrType::Uuid => {
            let s = string(value)?;
            let u = uuid::Uuid::parse_str(s)
                .map_err(|e| async_graphql::Error::new(format!("Invalid UUID: {e}")))?;
            Ok(AshValue::Uuid(u))
        }
        AttrType::String => Ok(AshValue::String(string(value)?.to_string())),
        AttrType::UtcDatetime { precision } => {
            let normalized = precision
                .normalize(string(value)?)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(normalized))
        }
        AttrType::Binary => {
            let binary = ash_core::Binary::parse(string(value)?)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(binary.encode()))
        }
        AttrType::Date => {
            let s = string(value)?;
            ash_core::Date::parse(s).map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::CiString => {
            let value = ash_core::CiString::parse(string(value)?)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(value.as_str().to_string()))
        }
        AttrType::Decimal => {
            let s = string(value)?;
            ash_core::Decimal::parse(s)
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(s.to_string()))
        }
        AttrType::Float => {
            let n = float(value)?;
            let float = ash_core::Float::parse(&n.to_string())
                .map_err(|err| async_graphql::Error::new(err.to_string()))?;
            Ok(AshValue::String(float.as_str().to_string()))
        }
        AttrType::Integer => match value {
            GqlValue::Number(n) => n
                .as_i64()
                .map(AshValue::Int)
                .ok_or_else(|| async_graphql::Error::new("internal: not an signed integer")),
            _ => Err(async_graphql::Error::new("internal: not an signed integer")),
        },
        AttrType::Boolean => match value {
            GqlValue::Boolean(b) => Ok(AshValue::Bool(*b)),
            _ => Err(async_graphql::Error::new("internal: not a boolean")),
        },
        AttrType::Inet => Ok(AshValue::String(inet_text(value)?)),
        AttrType::Vector { dimensions } => Ok(AshValue::String(vector_text(value, dimensions)?)),
        AttrType::Atom { one_of, name: type_name } => {
            let name = match (value, type_name) {
                (GqlValue::Enum(name), Some(_)) => name.as_str(),
                (GqlValue::String(name), _) => name.as_str(),
                _ => return Err(async_graphql::Error::new("internal: not an enum name")),
            };
            if let Some(matched) = one_of.iter().find(|&&s| s.eq_ignore_ascii_case(name)) {
                Ok(AshValue::String((*matched).to_string()))
            } else {
                Ok(AshValue::String(name.to_string()))
            }
        }
        // A map is JSON, its keys as the client sent them.
        AttrType::Map => Ok(graphql_value_to_ash_value(value)),
        // Cast as the type casts its JSON: each field, or the member given.
        AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. } => ash_core::input::value_input(ty, &value.clone().into_json()?).map_err(|e| async_graphql::Error::new(e.to_string())),
        // A list, each item as its type takes it.
        AttrType::Array { of } => match value {
            GqlValue::List(items) => items
                .iter()
                .map(|item| if matches!(item, GqlValue::Null) { Ok(AshValue::Null) } else { parse_input_value(item, *of) })
                .collect::<Result<Vec<_>, _>>()
                .map(AshValue::Array),
            // A single item stands for a list of it, as GraphQL coerces one.
            item => Ok(AshValue::Array(vec![parse_input_value(item, *of)?])),
        },
    }
}
