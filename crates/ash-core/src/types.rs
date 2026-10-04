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

/// How finely a UTC datetime keeps time, as Ash's `precision` constraint does:
/// `Second` truncates to whole seconds, `Microsecond` keeps six fractional digits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimePrecision {
    Second,
    Microsecond,
}

impl TimePrecision {
    /// `raw`, any RFC3339 timestamp, as this precision stores it: shifted to UTC,
    /// truncated, and written in one fixed-width form, so that text order is time
    /// order and every data layer stores the same instant the same way.
    pub fn normalize(self, raw: &str) -> Result<String> {
        let parsed = chrono::DateTime::parse_from_rfc3339(raw).map_err(|_| {
            Error::Invalid(format!(
                "invalid UTC datetime `{raw}`: expected an RFC3339 timestamp"
            ))
        })?;
        Ok(self.format(parsed.with_timezone(&chrono::Utc)))
    }

    /// `instant` in this precision's stored form. Every read of a timestamp writes one, so
    /// the digits are written directly rather than through a format string, which chrono
    /// parses on every call; a year without four digits takes chrono's form.
    pub fn format(self, instant: chrono::DateTime<chrono::Utc>) -> String {
        use chrono::{Datelike, Timelike};
        let year = instant.year();
        // A leap second, which chrono writes as `:60`, takes chrono's form too.
        if !(0..=9999).contains(&year) || instant.nanosecond() >= 1_000_000_000 {
            return match self {
                Self::Second => instant.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
                Self::Microsecond => {
                    let micros = instant.timestamp_subsec_micros().min(999_999);
                    format!("{}.{micros:06}Z", instant.format("%Y-%m-%dT%H:%M:%S"))
                }
            };
        }
        let time = instant.time();
        let mut out = String::with_capacity(27);
        push_digits(&mut out, year as u32, 4);
        for (separator, value) in [
            ('-', instant.month()),
            ('-', instant.day()),
            ('T', time.hour()),
            (':', time.minute()),
            (':', time.second()),
        ] {
            out.push(separator);
            push_digits(&mut out, value, 2);
        }
        if self == Self::Microsecond {
            out.push('.');
            push_digits(&mut out, instant.timestamp_subsec_micros().min(999_999), 6);
        }
        out.push('Z');
        out
    }
}

/// `value`'s last `width` decimal digits, zero-padded.
fn push_digits(out: &mut String, value: u32, width: u32) {
    for place in (0..width).rev() {
        out.push(char::from(b'0' + (value / 10u32.pow(place) % 10) as u8));
    }
}

macro_rules! utc_datetime_type {
    ($(#[$doc:meta])* $name:ident, $precision:expr) => {
        $(#[$doc])*
        ///
        /// Parsing accepts any RFC3339 timestamp and keeps the instant in UTC, so
        /// `10:00:00+02:00` is stored, compared and returned as `08:00:00Z`. Postgres
        /// columns use `timestamptz`; SQLite has no timestamp type, so the column is
        /// `TEXT`, which sorts correctly because the stored form has a fixed width.
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub const PRECISION: TimePrecision = $precision;

            pub fn parse(raw: &str) -> Result<Self> {
                Self::PRECISION.normalize(raw).map(Self)
            }

            /// The current time, at this type's precision.
            pub fn now() -> Self {
                Self(Self::PRECISION.format(chrono::Utc::now()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AshType for $name {
            const ATTR_TYPE: AttrType = AttrType::UtcDatetime {
                precision: $precision,
            };

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

        impl From<$name> for Value {
            fn from(value: $name) -> Self {
                value.to_value()
            }
        }
    };
}

utc_datetime_type!(
    /// A UTC instant to the second, as Ash's `:utc_datetime`. Fractional seconds are
    /// truncated.
    UtcDateTime,
    TimePrecision::Second
);

utc_datetime_type!(
    /// A UTC instant to the microsecond, as Ash's `:utc_datetime_usec`.
    UtcDateTimeUsec,
    TimePrecision::Microsecond
);

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

    /// The number, as an `f64`.
    pub fn value(&self) -> f64 {
        // `parse` only admits finite numbers, so the text always reads back.
        self.0.parse().unwrap_or_default()
    }
}

impl TryFrom<f64> for Float {
    type Error = Error;

    fn try_from(value: f64) -> Result<Self> {
        Self::parse(&value.to_string())
    }
}

/// A plain `f64` is a float attribute, as Ash's `:float` is an Elixir float. It's stored
/// as `Float`'s canonical text; a non-finite value fails validation when written.
impl AshType for f64 {
    const ATTR_TYPE: AttrType = AttrType::Float;

    fn to_value(&self) -> Value {
        Value::String(self.to_string())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Float::parse(s).map(|float| float.value()),
            Value::Int(n) => Ok(*n as f64),
            Value::Float(n) => Ok(*n),
            _ => Err(Error::Invalid("expected float".into())),
        }
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        value.to_value()
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

impl From<Decimal> for Value {
    fn from(value: Decimal) -> Self {
        value.to_value()
    }
}

impl From<Binary> for Value {
    fn from(value: Binary) -> Self {
        value.to_value()
    }
}

/// The text-backed types display as their stored text, so they can be formatted and
/// used in `concat(...)` calculations.
macro_rules! display_as_str {
    ($($ty:ty),*) => {$(
        impl std::fmt::Display for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }
    )*};
}

display_as_str!(UtcDateTime, UtcDateTimeUsec, Decimal, Float, CiString, Inet, Date);

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

/// Case-insensitive string. The Rust value keeps the original casing.
///
/// Postgres columns use `citext`. Migrations and `install()` create the extension
/// in `public` before the first citext column and never drop it, since other
/// schemas may use it. SQLite columns are `TEXT COLLATE NOCASE`, which ignores
/// case for ASCII letters only, and the in-memory store compares lowercased text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CiString(String);

impl CiString {
    pub fn parse(raw: &str) -> Result<Self> {
        Ok(Self(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<CiString> for Value {
    fn from(value: CiString) -> Self {
        value.to_value()
    }
}

impl AshType for CiString {
    const ATTR_TYPE: AttrType = AttrType::CiString;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected ci_string".into())),
        }
    }
}

/// IP address with an optional network prefix, such as `10.0.0.1` or `10.0.0.0/8`.
///
/// Postgres columns use `inet`. SQLite has no address type, so the column is `TEXT`.
/// The text form drops a prefix that covers the whole address, as Postgres does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inet(String);

impl Inet {
    pub fn parse(raw: &str) -> Result<Self> {
        let invalid = || Error::Invalid(format!("invalid inet `{raw}`: expected an IP address"));
        let (addr, prefix) = match raw.split_once('/') {
            Some((addr, prefix)) => (addr, Some(prefix)),
            None => (raw, None),
        };
        let addr: std::net::IpAddr = addr.parse().map_err(|_| invalid())?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(prefix) if prefix.bytes().all(|b| b.is_ascii_digit()) => {
                prefix.parse::<u8>().map_err(|_| invalid())?
            }
            Some(_) => return Err(invalid()),
            None => max,
        };
        if prefix > max {
            return Err(invalid());
        }
        Ok(Self(format_inet(addr, prefix)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Postgres text form of an inet value: the address, then `/prefix` unless it is the full width.
pub fn format_inet(addr: std::net::IpAddr, prefix: u8) -> String {
    let max = if addr.is_ipv4() { 32 } else { 128 };
    if prefix == max {
        addr.to_string()
    } else {
        format!("{addr}/{prefix}")
    }
}

impl From<Inet> for Value {
    fn from(value: Inet) -> Self {
        value.to_value()
    }
}

impl AshType for Inet {
    const ATTR_TYPE: AttrType = AttrType::Inet;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected inet".into())),
        }
    }
}

/// Embedding with exactly `N` finite `f32` values, stored as text like `[1,2.5,3]`.
///
/// Postgres columns use pgvector's `vector(N)`, which needs `CREATE EXTENSION vector`.
/// SQLite stores the same text in a `TEXT` column.
#[derive(Clone, Debug, PartialEq)]
pub struct Vector<const N: usize>(Vec<f32>);

impl<const N: usize> Vector<N> {
    pub fn new(values: impl Into<Vec<f32>>) -> Result<Self> {
        let values = values.into();
        check_vector(&values, N as u32)?;
        Ok(Self(values))
    }

    pub fn parse(raw: &str) -> Result<Self> {
        Self::new(parse_vector(raw)?)
    }

    pub fn as_slice(&self) -> &[f32] {
        &self.0
    }
}

// Values are always finite, since `new` rejects NaN, so equality is reflexive.
impl<const N: usize> Eq for Vector<N> {}

/// Parses pgvector text such as `[1,2.5,3]`.
pub fn parse_vector(raw: &str) -> Result<Vec<f32>> {
    let invalid = || Error::Invalid(format!("invalid vector `{raw}`: expected [1,2,3]"));
    let inner = raw
        .trim()
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(invalid)?;
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    inner
        .split(',')
        .map(|part| part.trim().parse::<f32>().map_err(|_| invalid()))
        .collect()
}

/// Checks that a vector has `dimensions` finite values.
pub fn check_vector(values: &[f32], dimensions: u32) -> Result<()> {
    if values.len() != dimensions as usize {
        return Err(Error::Invalid(format!(
            "expected a vector with {dimensions} dimensions, got {}",
            values.len()
        )));
    }
    if values.iter().any(|value| !value.is_finite()) {
        return Err(Error::Invalid("vector values must be finite".into()));
    }
    Ok(())
}

/// pgvector text form of `values`, such as `[1,2.5,3]`. Each value is the shortest text
/// that reads back as the same `f32`, with an exponent for very large or small values.
pub fn format_vector(values: &[f32]) -> String {
    let parts: Vec<String> = values.iter().map(|value| format_f32(*value)).collect();
    format!("[{}]", parts.join(","))
}

fn format_f32(value: f32) -> String {
    let text = format!("{value:?}");
    match text.strip_suffix(".0") {
        Some(whole) => whole.to_string(),
        None => text,
    }
}

impl<const N: usize> From<Vector<N>> for Value {
    fn from(value: Vector<N>) -> Self {
        value.to_value()
    }
}

impl<const N: usize> AshType for Vector<N> {
    const ATTR_TYPE: AttrType = AttrType::Vector {
        dimensions: N as u32,
    };

    fn to_value(&self) -> Value {
        Value::String(format_vector(&self.0))
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            _ => Err(Error::Invalid("expected vector".into())),
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
    // Postgres dates have no year 0, and 4 digits keep the text sortable.
    let Ok(year @ 1..) = raw[..4].parse::<i32>() else {
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

impl AshType for Float {
    const ATTR_TYPE: AttrType = AttrType::Float;

    fn to_value(&self) -> Value {
        Value::String(self.0.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::String(s) => Self::parse(s),
            Value::Float(n) => Self::parse(&n.to_string()),
            Value::Int(n) => Self::parse(&n.to_string()),
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

/// Compares two decimal strings exactly, or `None` if either is not a decimal.
pub fn compare_decimal(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;

    fn split(raw: &str) -> Option<(bool, String, String)> {
        if !is_decimal(raw) {
            return None;
        }
        let negative = raw.starts_with('-');
        let rest = raw.trim_start_matches(['+', '-']);
        let (whole, fraction) = rest.split_once('.').unwrap_or((rest, ""));
        let whole = whole.trim_start_matches('0').to_string();
        let fraction = fraction.trim_end_matches('0').to_string();
        let zero = whole.is_empty() && fraction.is_empty();
        Some((negative && !zero, whole, fraction))
    }

    let (a_negative, a_whole, a_fraction) = split(a)?;
    let (b_negative, b_whole, b_fraction) = split(b)?;
    let magnitude = a_whole
        .len()
        .cmp(&b_whole.len())
        .then_with(|| a_whole.cmp(&b_whole))
        .then_with(|| a_fraction.cmp(&b_fraction));
    Some(match (a_negative, b_negative) {
        (false, true) => Ordering::Greater,
        (true, false) => Ordering::Less,
        (false, false) => magnitude,
        (true, true) => magnitude.reverse(),
    })
}

/// Compares two stored values the way their column type does in SQL: `Float` and
/// `Decimal` numerically and `CiString` ignoring case. Other types use [`Value`]'s order.
/// The canonical text of `raw` for types with more than one spelling of a value, such as
/// `10.0.0.1/32` for `10.0.0.1` or `[1.0, 2]` for `[1,2]`. Postgres canonicalizes these
/// itself, so storing and comparing the canonical text keeps the other stores in step.
pub fn canonical_text(ty: AttrType, raw: &str) -> Option<String> {
    match ty {
        AttrType::UtcDatetime { precision } => precision.normalize(raw).ok(),
        AttrType::Inet => Inet::parse(raw).ok().map(|inet| inet.as_str().to_string()),
        AttrType::Vector { .. } => parse_vector(raw).ok().map(|values| format_vector(&values)),
        AttrType::Float => Float::parse(raw).ok().map(|float| float.as_str().to_string()),
        _ => None,
    }
}

pub fn compare_typed(ty: Option<AttrType>, a: &Value, b: &Value) -> std::cmp::Ordering {
    if let (Value::String(x), Value::String(y)) = (a, b) {
        match ty {
            Some(ty @ (AttrType::Inet | AttrType::Vector { .. } | AttrType::UtcDatetime { .. })) => {
                let canonical = |raw: &str| canonical_text(ty, raw).unwrap_or_else(|| raw.to_string());
                return canonical(x).cmp(&canonical(y));
            }
            Some(AttrType::Float) => {
                if let (Ok(x), Ok(y)) = (x.parse::<f64>(), y.parse::<f64>())
                    && let Some(order) = x.partial_cmp(&y)
                {
                    return order;
                }
            }
            Some(AttrType::Decimal) => {
                if let Some(order) = compare_decimal(x, y) {
                    return order;
                }
            }
            Some(AttrType::CiString) => return x.to_lowercase().cmp(&y.to_lowercase()),
            _ => {}
        }
    }
    a.cmp(b)
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

/// A map of values, as Ash's `:map`.
impl AshType for crate::value::FieldMap {
    const ATTR_TYPE: AttrType = AttrType::Map;

    fn to_value(&self) -> Value {
        Value::Map(self.clone())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Map(map) => Ok(map.clone()),
            other => Err(Error::Invalid(format!("expected a map, got {other}"))),
        }
    }
}

/// A list of one type, as Ash's `{:array, type}`.
impl<T: AshType> AshType for Vec<T> {
    const ATTR_TYPE: AttrType = AttrType::Array;

    fn to_value(&self) -> Value {
        Value::Array(self.iter().map(AshType::to_value).collect())
    }

    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Array(items) => items.iter().map(T::from_value).collect(),
            other => Err(Error::Invalid(format!("expected a list, got {other}"))),
        }
    }
}

#[cfg(test)]
mod network_and_vector_tests {
    use super::{Inet, Vector};

    #[test]
    fn inet_normalizes_full_prefixes_and_rejects_bad_input() {
        assert_eq!(Inet::parse("10.0.0.1").unwrap().as_str(), "10.0.0.1");
        assert_eq!(Inet::parse("10.0.0.1/32").unwrap().as_str(), "10.0.0.1");
        assert_eq!(Inet::parse("10.0.0.0/8").unwrap().as_str(), "10.0.0.0/8");
        assert_eq!(
            Inet::parse("2001:DB8:0:0:0:0:0:1/64").unwrap().as_str(),
            "2001:db8::1/64"
        );
        assert_eq!(Inet::parse("::1/128").unwrap().as_str(), "::1");
        for bad in ["10.0.0", "10.0.0.1/33", "::1/129", "10.0.0.1/", "10.0.0.1/+8", "host"] {
            assert!(Inet::parse(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn vector_checks_dimensions_and_round_trips_text() {
        let v = Vector::<3>::new(vec![1.0, 2.5, -3.0]).unwrap();
        assert_eq!(super::format_vector(v.as_slice()), "[1,2.5,-3]");
        let extremes = [3.4e38, 1e-45, 0.1, 1e20, -0.0];
        assert_eq!(super::format_vector(&extremes), "[3.4e38,1e-45,0.1,1e20,-0]");
        assert_eq!(super::parse_vector("[3.4e38,1e-45,0.1,1e20,-0]").unwrap(), extremes);
        assert_eq!(Vector::<3>::parse("[1, 2.5, -3]").unwrap(), v);
        assert!(Vector::<3>::new(vec![1.0, 2.0]).is_err());
        assert!(Vector::<2>::new(vec![1.0, f32::NAN]).is_err());
        assert!(Vector::<2>::parse("1,2").is_err());
        assert!(Vector::<2>::parse("[1,x]").is_err());
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
    fn stored_datetimes_are_written_as_chrono_writes_them() {
        let chrono_form = |precision: TimePrecision, instant: chrono::DateTime<chrono::Utc>| match precision {
            TimePrecision::Second => instant.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            TimePrecision::Microsecond => {
                format!("{}.{:06}Z", instant.format("%Y-%m-%dT%H:%M:%S"), instant.timestamp_subsec_micros())
            }
        };
        let instants = [
            "0000-01-01T00:00:00Z",
            "0999-12-31T23:59:59.000001Z",
            "1970-01-01T00:00:00Z",
            "2026-10-02T09:05:07.123456Z",
            "9999-12-31T23:59:59.999999Z",
        ]
        .into_iter()
        .map(|raw| chrono::DateTime::parse_from_rfc3339(raw).unwrap().with_timezone(&chrono::Utc))
        .chain((0..1000).map(|i| chrono::DateTime::from_timestamp_micros(i * 7_919_237_123_457 - 3_000_000_000_000_000).unwrap()));
        for instant in instants {
            for precision in [TimePrecision::Second, TimePrecision::Microsecond] {
                assert_eq!(precision.format(instant), chrono_form(precision, instant), "{instant:?}");
            }
        }
    }

    #[test]
    fn utc_datetimes_are_stored_in_utc_at_their_precision() {
        let at = |raw: &str| UtcDateTime::parse(raw).unwrap().as_str().to_string();
        let at_usec = |raw: &str| UtcDateTimeUsec::parse(raw).unwrap().as_str().to_string();
        // Offsets shift to UTC, across a day boundary too.
        assert_eq!(at("2187-02-12T01:30:00+02:00"), "2187-02-11T23:30:00Z");
        assert_eq!(at("2187-02-11T20:00:00-05:00"), "2187-02-12T01:00:00Z");
        // Seconds truncate; microseconds keep six digits, padded or truncated.
        assert_eq!(at("2187-02-12T08:00:00.987Z"), "2187-02-12T08:00:00Z");
        assert_eq!(at_usec("2187-02-12T08:00:00.5Z"), "2187-02-12T08:00:00.500000Z");
        assert_eq!(at_usec("2187-02-12T08:00:00Z"), "2187-02-12T08:00:00.000000Z");
        assert_eq!(at_usec("2187-02-12T08:00:00.1234567Z"), "2187-02-12T08:00:00.123456Z");
        // The stored form sorts as time does.
        let mut instants = ["2187-02-12T08:00:00.5Z", "2187-02-12T09:00:00+02:00", "2187-02-12T08:00:00Z"]
            .map(|raw| UtcDateTimeUsec::parse(raw).unwrap());
        instants.sort();
        assert_eq!(
            instants.map(|instant| instant.as_str().to_string()),
            [
                "2187-02-12T07:00:00.000000Z",
                "2187-02-12T08:00:00.000000Z",
                "2187-02-12T08:00:00.500000Z",
            ]
        );
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
    fn typed_comparison_is_numeric_and_case_insensitive() {
        use super::{compare_decimal, compare_typed};
        use crate::resource::AttrType;
        use std::cmp::Ordering;

        let s = |v: &str| Value::String(v.to_string());
        assert_eq!(compare_typed(Some(AttrType::Float), &s("10"), &s("9.5")), Ordering::Greater);
        assert_eq!(compare_typed(None, &s("10"), &s("9.5")), Ordering::Less);
        assert_eq!(compare_typed(Some(AttrType::Float), &s("1.50"), &s("1.5")), Ordering::Equal);
        assert_eq!(
            compare_typed(Some(AttrType::CiString), &s("Ada"), &s("ada")),
            Ordering::Equal
        );
        for (a, b, order) in [
            ("10", "9.99", Ordering::Greater),
            ("-10", "-9.99", Ordering::Less),
            ("0.10", "0.1", Ordering::Equal),
            ("-0", "0.000", Ordering::Equal),
            ("007.5", "7.50", Ordering::Equal),
            ("-1", "1", Ordering::Less),
            ("123456789012345678901234567890.1", "123456789012345678901234567890.09", Ordering::Greater),
        ] {
            assert_eq!(compare_decimal(a, b), Some(order), "{a} vs {b}");
        }
        assert_eq!(compare_decimal("x", "1"), None);
    }

    #[test]
    fn date_rejects_impossible_days() {
        assert_eq!(Date::parse("2024-02-29").unwrap().as_str(), "2024-02-29");
        assert!(Date::parse("2023-02-29").is_err());
        assert!(Date::parse("2024-04-31").is_err());
        assert!(Date::parse("2024-1-02").is_err());
        assert!(Date::parse("2024-01-02T00:00:00Z").is_err());
        assert!(Date::parse("0000-01-01").is_err());
        assert!(Date::parse("0001-01-01").is_ok());
    }

    #[test]
    fn ci_string_keeps_the_original_casing() {
        let email = CiString::parse("Ada@Example.com").unwrap();
        assert_eq!(email.as_str(), "Ada@Example.com");
        assert_eq!(email.to_value(), Value::String("Ada@Example.com".into()));
    }

    #[test]
    fn binary_round_trips_base64() {
        let bytes = Binary::from_bytes(b"hello".to_vec());
        assert_eq!(bytes.encode(), "aGVsbG8=");
        assert_eq!(Binary::parse("aGVsbG8=").unwrap().as_bytes(), b"hello");
        assert!(Binary::parse("!!!!").is_err());
    }
}
