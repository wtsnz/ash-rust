use base64::Engine;
use base64::engine::general_purpose::STANDARD;

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

macro_rules! impl_ash_int {
    ($($t:ty),*) => {
        $(
            impl AshType for $t {
                const ATTR_TYPE: AttrType = AttrType::Integer;

                fn to_value(&self) -> Value {
                    Value::Int(*self as i64)
                }

                fn from_value(value: &Value) -> Result<Self> {
                    match value {
                        Value::Int(n) => (*n).try_into().map_err(|_| {
                            Error::Invalid(format!(
                                "integer {n} does not fit in {}",
                                stringify!($t)
                            ))
                        }),
                        _ => Err(Error::Invalid("expected integer".into())),
                    }
                }
            }
        )*
    };
}

impl_ash_int!(i8, i16, i32, isize, u8, u16, u32, u64, usize, i128, u128);

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

/// UTC instant stored as an RFC3339 string.
///
/// Postgres columns use `timestamptz`. SQLite has no timestamp type, so the column is `TEXT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UtcDateTime(String);

impl UtcDateTime {
    pub fn parse(raw: &str) -> Result<Self> {
        if is_rfc3339(raw) {
            Ok(Self(raw.to_string()))
        } else {
            Err(Error::Invalid(format!(
                "invalid UTC datetime `{raw}`: expected an RFC3339 timestamp"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AshType for UtcDateTime {
    const ATTR_TYPE: AttrType = AttrType::UtcDatetime;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected UTC datetime".into())),
        }
    }
}

/// Base-10 number stored as a string so trailing zeros survive a round trip.
///
/// Postgres columns use `numeric`. SQLite columns use `NUMERIC`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decimal(String);

impl Decimal {
    pub fn parse(raw: &str) -> Result<Self> {
        if is_decimal(raw) {
            Ok(Self(raw.to_string()))
        } else {
            Err(Error::Invalid(format!(
                "invalid decimal `{raw}`: expected a base-10 number"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// IEEE-754 binary64 number stored as its canonical decimal text.
///
/// Postgres columns use `double precision`. SQLite columns use `REAL`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Float(String);

impl Float {
    pub fn parse(raw: &str) -> Result<Self> {
        let parsed: f64 = raw.parse().map_err(|_| {
            Error::Invalid(format!("invalid float `{raw}`: expected a finite number"))
        })?;
        if !parsed.is_finite() {
            return Err(Error::Invalid(format!(
                "invalid float `{raw}`: expected a finite number"
            )));
        }
        Ok(Self(parsed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Opaque bytes stored as standard base64.
///
/// Postgres columns use `bytea`. SQLite columns use `BLOB`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binary(Vec<u8>);

impl Binary {
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn parse(raw: &str) -> Result<Self> {
        STANDARD
            .decode(raw)
            .map(Self)
            .map_err(|err| Error::Invalid(format!("invalid base64 `{raw}`: {err}")))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn encode(&self) -> String {
        STANDARD.encode(&self.0)
    }
}

impl From<Binary> for Value {
    fn from(value: Binary) -> Self {
        value.to_value()
    }
}

impl crate::value::IntoOption<Binary> for Binary {
    fn into_option(self) -> Option<Binary> {
        Some(self)
    }
}

impl AshType for Binary {
    const ATTR_TYPE: AttrType = AttrType::Binary;

    fn to_value(&self) -> Value {
        Value::String(self.encode())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected binary".into())),
        }
    }
}

/// Calendar date stored as `YYYY-MM-DD`.
///
/// Postgres columns use `date`. SQLite has no date type, so the column is `TEXT`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Date(String);

impl Date {
    pub fn parse(raw: &str) -> Result<Self> {
        if is_calendar_date(raw) {
            Ok(Self(raw.to_string()))
        } else {
            Err(Error::Invalid(format!(
                "invalid date `{raw}`: expected YYYY-MM-DD"
            )))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<Date> for Value {
    fn from(value: Date) -> Self {
        value.to_value()
    }
}

impl crate::value::IntoOption<Date> for Date {
    fn into_option(self) -> Option<Date> {
        Some(self)
    }
}

impl AshType for Date {
    const ATTR_TYPE: AttrType = AttrType::Date;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected date".into())),
        }
    }
}

fn is_calendar_date(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => true,
        _ => b.is_ascii_digit(),
    }) {
        return false;
    }
    let Ok(year) = raw[..4].parse::<i32>() else {
        return false;
    };
    let Ok(month) = raw[5..7].parse::<u8>() else {
        return false;
    };
    let Ok(day) = raw[8..10].parse::<u8>() else {
        return false;
    };
    let max = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => return false,
    };
    (1..=max).contains(&day)
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

impl From<Float> for Value {
    fn from(value: Float) -> Self {
        value.to_value()
    }
}

impl crate::value::IntoOption<Float> for Float {
    fn into_option(self) -> Option<Float> {
        Some(self)
    }
}

impl AshType for Float {
    const ATTR_TYPE: AttrType = AttrType::Float;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected float".into())),
        }
    }
}

impl AshType for Decimal {
    const ATTR_TYPE: AttrType = AttrType::Decimal;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected decimal".into())),
        }
    }
}

fn is_rfc3339(raw: &str) -> bool {
    let (body, offset) = if let Some(body) = raw.strip_suffix('Z') {
        (body, true)
    } else if raw.len() >= 6 {
        let split = raw.len() - 6;
        let (body, off) = raw.split_at(split);
        let bytes = off.as_bytes();
        let signed = bytes[0] == b'+' || bytes[0] == b'-';
        let hours = off[1..3].parse::<u8>().ok();
        let minutes = off[4..6].parse::<u8>().ok();
        (
            body,
            signed
                && bytes[3] == b':'
                && hours.is_some_and(|h| h <= 23)
                && minutes.is_some_and(|m| m <= 59),
        )
    } else {
        ("", false)
    };
    if !offset {
        return false;
    }
    let (date, fraction) = match body.split_once('.') {
        Some((date, fraction)) => {
            if fraction.is_empty() || !fraction.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            (date, true)
        }
        None => (body, false),
    };
    let _ = fraction;
    let bytes = date.as_bytes();
    if bytes.len() != 19
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    let year = date[..4].parse::<u16>().is_ok();
    let month = date[5..7]
        .parse::<u8>()
        .ok()
        .is_some_and(|n| (1..=12).contains(&n));
    let day = date[8..10]
        .parse::<u8>()
        .ok()
        .is_some_and(|n| (1..=31).contains(&n));
    let hour = date[11..13].parse::<u8>().ok().is_some_and(|n| n <= 23);
    let minute = date[14..16].parse::<u8>().ok().is_some_and(|n| n <= 59);
    let second = date[17..19].parse::<u8>().ok().is_some_and(|n| n <= 60);
    year && month
        && day
        && hour
        && minute
        && second
        && date.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 | 10 | 13 | 16 => true,
            _ => b.is_ascii_digit(),
        })
}

fn is_decimal(raw: &str) -> bool {
    let rest = raw.strip_prefix(['+', '-']).unwrap_or(raw);
    if rest.is_empty() {
        return false;
    }
    let mut parts = rest.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if parts.next().is_some() {
        return false;
    }
    let whole_ok = whole.bytes().all(|b| b.is_ascii_digit());
    let fraction_ok = match fraction {
        Some(fraction) => !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()),
        None => true,
    };
    whole_ok && fraction_ok && !(whole.is_empty() && fraction.is_none())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_datetime_accepts_rfc3339() {
        assert!(UtcDateTime::parse("2024-01-02T03:04:05Z").is_ok());
        assert!(UtcDateTime::parse("2024-01-02T03:04:05.123Z").is_ok());
        assert!(UtcDateTime::parse("2024-01-02T03:04:05+00:00").is_ok());
        assert!(UtcDateTime::parse("2024-01-02").is_err());
        assert!(UtcDateTime::parse("2024-01-02T03:04:05").is_err());
        assert!(UtcDateTime::parse("nope").is_err());
    }

    #[test]
    fn decimal_keeps_the_written_scale() {
        let amount = Decimal::parse("12.50").unwrap();
        assert_eq!(amount.as_str(), "12.50");
        assert_eq!(amount.to_value(), Value::String("12.50".into()));
        assert!(Decimal::parse("-0.5").is_ok());
        assert!(Decimal::parse(".5").is_ok());
        assert!(Decimal::parse("12.").is_err());
        assert!(Decimal::parse("1e2").is_err());
        assert!(Decimal::parse("").is_err());
    }

    #[test]
    fn float_canonicalizes_finite_numbers() {
        assert_eq!(Float::parse("1.50").unwrap().as_str(), "1.5");
        assert_eq!(Float::parse("-0").unwrap().as_str(), "-0");
        assert!(Float::parse("inf").is_err());
        assert!(Float::parse("nan").is_err());
        assert!(Float::parse("nope").is_err());
    }

    #[test]
    fn date_rejects_impossible_days() {
        assert_eq!(Date::parse("2024-02-29").unwrap().as_str(), "2024-02-29");
        assert!(Date::parse("2023-02-29").is_err());
        assert!(Date::parse("2024-04-31").is_err());
        assert!(Date::parse("2024-1-02").is_err());
        assert!(Date::parse("2024-01-02T00:00:00Z").is_err());
    }

    #[test]
    fn binary_round_trips_base64() {
        let bytes = Binary::from_bytes(b"hello".to_vec());
        assert_eq!(bytes.encode(), "aGVsbG8=");
        assert_eq!(Binary::parse("aGVsbG8=").unwrap().as_bytes(), b"hello");
        assert!(Binary::parse("!!!!").is_err());
    }
}
