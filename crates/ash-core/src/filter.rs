use std::cmp::Ordering;

use crate::resource::{AttrType, ResourceDef};
use crate::value::{FieldMap, Value};

#[derive(Clone, Debug, PartialEq)]
pub enum Filter {
    True,
    False,
    Eq(String, Value),
    Ne(String, Value),
    Gt(String, Value),
    Gte(String, Value),
    Lt(String, Value),
    Lte(String, Value),
    In(String, Vec<Value>),
    IsNil(String),
    /// Text contains the substring. Case-insensitive on `CiString` attributes.
    Contains(String, String),
    /// Text starts with the prefix. Case-insensitive on `CiString` attributes.
    StartsWith(String, String),
    /// Text ends with the suffix. Case-insensitive on `CiString` attributes.
    EndsWith(String, String),
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
    Related {
        relationship: String,
        filter: Box<Filter>,
    },
}

impl Filter {
    pub fn related(relationship: impl Into<String>, filter: Filter) -> Self {
        Self::Related {
            relationship: relationship.into(),
            filter: Box::new(filter),
        }
    }

    pub fn eq(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Eq(field.into(), value.into())
    }

    pub fn ne(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Ne(field.into(), value.into())
    }

    pub fn is_nil(field: impl Into<String>) -> Self {
        Self::IsNil(field.into())
    }

    pub fn gt(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Gt(field.into(), value.into())
    }

    pub fn gte(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Gte(field.into(), value.into())
    }

    pub fn lt(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Lt(field.into(), value.into())
    }

    pub fn lte(field: impl Into<String>, value: impl Into<Value>) -> Self {
        Self::Lte(field.into(), value.into())
    }

    pub fn contains(field: impl Into<String>, substring: impl Into<String>) -> Self {
        Self::Contains(field.into(), substring.into())
    }

    pub fn starts_with(field: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self::StartsWith(field.into(), prefix.into())
    }

    pub fn ends_with(field: impl Into<String>, suffix: impl Into<String>) -> Self {
        Self::EndsWith(field.into(), suffix.into())
    }

    pub fn in_list(field: impl Into<String>, values: impl IntoIterator<Item = impl Into<Value>>) -> Self {
        Self::In(field.into(), values.into_iter().map(Into::into).collect())
    }

    pub fn collect_fields<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::True | Self::False => {}
            Self::Eq(field, _)
            | Self::Ne(field, _)
            | Self::Gt(field, _)
            | Self::Gte(field, _)
            | Self::Lt(field, _)
            | Self::Lte(field, _)
            | Self::In(field, _)
            | Self::IsNil(field)
            | Self::Contains(field, _)
            | Self::StartsWith(field, _)
            | Self::EndsWith(field, _) => out.push(field),
            Self::And(parts) | Self::Or(parts) => {
                for part in parts {
                    part.collect_fields(out);
                }
            }
            Self::Not(inner) => inner.collect_fields(out),
            Self::Related { filter, .. } => filter.collect_fields(out),
        }
    }

    pub fn and(parts: impl IntoIterator<Item = Filter>) -> Self {
        let mut out = Vec::new();
        for part in parts {
            match part {
                Self::True => {}
                Self::False => return Self::False,
                Self::And(mut nested) => out.append(&mut nested),
                other => out.push(other),
            }
        }
        match out.len() {
            0 => Self::True,
            1 => out.remove(0),
            _ => Self::And(out),
        }
    }

    pub fn or(parts: impl IntoIterator<Item = Filter>) -> Self {
        let mut out = Vec::new();
        for part in parts {
            match part {
                Self::False => {}
                Self::True => return Self::True,
                Self::Or(mut nested) => out.append(&mut nested),
                other => out.push(other),
            }
        }
        match out.len() {
            0 => Self::False,
            1 => out.remove(0),
            _ => Self::Or(out),
        }
    }

    /// Evaluates the filter against one record. Without attribute types, text
    /// matching here is case-sensitive even for `CiString` fields; use
    /// [`Filter::matches_on`] when the resource is known. A comparison with a null
    /// field is unknown, as in SQL, so `NOT` over it does not match either.
    pub fn matches(&self, fields: &FieldMap) -> bool {
        self.eval(None, fields) == Some(true)
    }

    /// Evaluates the filter against one record of `resource`, so text filters on
    /// `CiString` attributes and calculations ignore case as they do in the data layers.
    pub fn matches_on(&self, resource: &ResourceDef, fields: &FieldMap) -> bool {
        self.eval(Some(resource), fields) == Some(true)
    }

    /// SQL's three-valued result, with `None` for unknown.
    fn eval(&self, resource: Option<&ResourceDef>, fields: &FieldMap) -> Option<bool> {
        let present = |field: &str| fields.get(field).filter(|got| !got.is_null());
        let text = |field: &str, needle: &str, test: fn(&str, &str) -> bool| {
            let ci = resource.is_some_and(|resource| {
                resource
                    .attribute(field)
                    .map(|attr| attr.ty)
                    .or_else(|| resource.calculation(field).map(|calc| calc.ty))
                    == Some(AttrType::CiString)
            });
            present(field).map(|got| text_matches(Some(got), needle, ci, test))
        };
        match self {
            Self::True => Some(true),
            Self::False => Some(false),
            Self::Eq(field, value) if value.is_null() => Some(present(field).is_none()),
            Self::Ne(field, value) if value.is_null() => Some(present(field).is_some()),
            Self::Eq(field, value) => present(field).map(|got| got == value),
            Self::Ne(field, value) => present(field).map(|got| got != value),
            Self::Gt(field, value) => compare(fields.get(field), value, Ordering::Greater, false),
            Self::Gte(field, value) => compare(fields.get(field), value, Ordering::Greater, true),
            Self::Lt(field, value) => compare(fields.get(field), value, Ordering::Less, false),
            Self::Lte(field, value) => compare(fields.get(field), value, Ordering::Less, true),
            Self::In(_, values) if values.is_empty() => Some(false),
            Self::In(field, values) => {
                in_list(present(field), values, |got, value| got == value)
            }
            Self::IsNil(field) => Some(present(field).is_none()),
            Self::Contains(field, needle) => text(field, needle, |text, needle| text.contains(needle)),
            Self::StartsWith(field, needle) => {
                text(field, needle, |text, needle| text.starts_with(needle))
            }
            Self::EndsWith(field, needle) => text(field, needle, |text, needle| text.ends_with(needle)),
            Self::And(parts) => all_of(parts.iter().map(|part| part.eval(resource, fields))),
            Self::Or(parts) => any_of(parts.iter().map(|part| part.eval(resource, fields))),
            Self::Not(inner) => inner.eval(resource, fields).map(|matched| !matched),
            Self::Related { relationship, filter } => {
                let destination = resource
                    .and_then(|resource| resource.relationship(relationship))
                    .map(|rel| (rel.destination)());
                let matches = |m: &FieldMap| filter.eval(destination, m) == Some(true);
                Some(match fields.get(relationship) {
                    Some(Value::Map(m)) => matches(m),
                    Some(Value::Array(arr)) => arr.iter().any(|v| match v {
                        Value::Map(m) => matches(m),
                        _ => false,
                    }),
                    _ => true,
                })
            }
        }
    }
}

/// SQL `AND` over three-valued results: false wins, then unknown.
pub fn all_of(results: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for result in results {
        match result {
            Some(false) => return Some(false),
            None => unknown = true,
            Some(true) => {}
        }
    }
    if unknown { None } else { Some(true) }
}

/// SQL `OR` over three-valued results: true wins, then unknown.
pub fn any_of(results: impl IntoIterator<Item = Option<bool>>) -> Option<bool> {
    let mut unknown = false;
    for result in results {
        match result {
            Some(true) => return Some(true),
            None => unknown = true,
            Some(false) => {}
        }
    }
    if unknown { None } else { Some(false) }
}

/// SQL `IN`: unknown for a null value, or when nothing matched but the list holds a null.
pub fn in_list(
    got: Option<&Value>,
    values: &[Value],
    same: impl Fn(&Value, &Value) -> bool,
) -> Option<bool> {
    let got = got.filter(|got| !got.is_null())?;
    if values.iter().any(|value| !value.is_null() && same(got, value)) {
        Some(true)
    } else if values.iter().any(Value::is_null) {
        None
    } else {
        Some(false)
    }
}

/// Applies a text match to a stored value. Non-string and null values never match.
pub fn text_matches(
    got: Option<&Value>,
    needle: &str,
    case_insensitive: bool,
    test: impl Fn(&str, &str) -> bool,
) -> bool {
    let Some(Value::String(text)) = got else {
        return false;
    };
    if case_insensitive {
        test(&text.to_lowercase(), &needle.to_lowercase())
    } else {
        test(text, needle)
    }
}

fn compare(got: Option<&Value>, rhs: &Value, direction: Ordering, equal_ok: bool) -> Option<bool> {
    let got = got.filter(|got| !got.is_null())?;
    if rhs.is_null() {
        return None;
    }
    Some(match got.cmp(rhs) {
        Ordering::Equal => equal_ok,
        order => order == direction,
    })
}

impl std::ops::Not for Filter {
    type Output = Self;

    fn not(self) -> Self {
        match self {
            Self::True => Self::False,
            Self::False => Self::True,
            other => Self::Not(Box::new(other)),
        }
    }
}

impl std::ops::BitAnd for Filter {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self {
        Self::and([self, rhs])
    }
}

impl std::ops::BitAnd<&Filter> for Filter {
    type Output = Self;

    fn bitand(self, rhs: &Filter) -> Self {
        Self::and([self, rhs.clone()])
    }
}

impl std::ops::BitAnd<Filter> for &Filter {
    type Output = Filter;

    fn bitand(self, rhs: Filter) -> Filter {
        Filter::and([self.clone(), rhs])
    }
}

impl std::ops::BitAnd<&Filter> for &Filter {
    type Output = Filter;

    fn bitand(self, rhs: &Filter) -> Filter {
        Filter::and([self.clone(), rhs.clone()])
    }
}

impl std::ops::BitOr for Filter {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self::or([self, rhs])
    }
}

impl std::ops::BitOr<&Filter> for Filter {
    type Output = Self;

    fn bitor(self, rhs: &Filter) -> Self {
        Self::or([self, rhs.clone()])
    }
}

impl std::ops::BitOr<Filter> for &Filter {
    type Output = Filter;

    fn bitor(self, rhs: Filter) -> Filter {
        Filter::or([self.clone(), rhs])
    }
}

impl std::ops::BitOr<&Filter> for &Filter {
    type Output = Filter;

    fn bitor(self, rhs: &Filter) -> Filter {
        Filter::or([self.clone(), rhs.clone()])
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::Filter;
    use crate::fields;
    use crate::value::Value;

    #[test]
    fn eq_and_or() {
        let id = Uuid::nil();
        let open = fields! {
            "id" => id,
            "status" => "open",
        };

        assert!(Filter::eq("status", "open").matches(&open));
        assert!(!Filter::eq("status", "closed").matches(&open));
        assert!(
            Filter::or([Filter::eq("status", "closed"), Filter::eq("status", "open")])
                .matches(&open)
        );
        assert!(
            !Filter::and([Filter::eq("status", "open"), Filter::eq("status", "closed")])
                .matches(&open)
        );
    }

    #[test]
    fn nil_matches_missing_and_null() {
        let empty = fields! {};
        let null = fields! { "assignee_id" => Option::<Uuid>::None };
        let set = fields! { "assignee_id" => Uuid::nil() };

        assert!(Filter::is_nil("assignee_id").matches(&empty));
        assert!(Filter::is_nil("assignee_id").matches(&null));
        assert!(!Filter::is_nil("assignee_id").matches(&set));
    }

    #[test]
    fn ne_matches_sql_null_semantics() {
        let empty = fields! {};
        let null = fields! { "status" => Value::Null };
        let draft = fields! { "status" => "draft" };
        let published = fields! { "status" => "published" };

        let ne_published = Filter::ne("status", "published");

        // SQL: NULL <> 'published' is false
        assert!(!ne_published.matches(&empty));
        assert!(!ne_published.matches(&null));

        // Non-null unequal matches:
        assert!(ne_published.matches(&draft));

        // Equal does not match:
        assert!(!ne_published.matches(&published));

        // Filter::ne on null matches non-null (IS NOT NULL):
        let not_null = Filter::ne("status", Value::Null);
        assert!(!not_null.matches(&empty));
        assert!(!not_null.matches(&null));
        assert!(not_null.matches(&draft));
        assert!(not_null.matches(&published));
    }

    #[test]
    fn and_or_collapse_true_false() {
        assert_eq!(
            Filter::and([Filter::True, Filter::eq("a", "b")]),
            Filter::eq("a", "b")
        );
        assert_eq!(
            Filter::and([Filter::False, Filter::eq("a", "b")]),
            Filter::False
        );
        assert_eq!(
            Filter::or([Filter::False, Filter::eq("a", "b")]),
            Filter::eq("a", "b")
        );
        assert_eq!(
            Filter::or([Filter::True, Filter::eq("a", "b")]),
            Filter::True
        );
    }

    #[test]
    fn gt_skips_null_and_compares_ints() {
        let row = fields! { "n" => 10_i64 };
        assert!(Filter::gt("n", 5_i64).matches(&row));
        assert!(!Filter::gt("n", 10_i64).matches(&row));
        assert!(Filter::gte("n", 10_i64).matches(&row));
        assert!(!Filter::gt("missing", 1_i64).matches(&row));
    }

    #[test]
    fn text_matching_is_case_sensitive_and_skips_null() {
        let row = fields! { "title" => "Printer on fire", "notes" => Value::Null };

        assert!(Filter::contains("title", "on fi").matches(&row));
        assert!(!Filter::contains("title", "ON FI").matches(&row));
        assert!(Filter::starts_with("title", "Printer").matches(&row));
        assert!(!Filter::starts_with("title", "fire").matches(&row));
        assert!(Filter::ends_with("title", "fire").matches(&row));
        assert!(!Filter::ends_with("title", "Printer").matches(&row));
        assert!(Filter::contains("title", "").matches(&row));
        assert!(!Filter::contains("notes", "").matches(&row));
        assert!(!Filter::contains("missing", "").matches(&row));
    }

    #[test]
    fn matches_on_ignores_case_for_ci_string_attributes() {
        use crate::resource::{AttrType, AttributeDef, ResourceDef};

        static ATTRS: &[AttributeDef] = &[
            AttributeDef::uuid_pk("id"),
            AttributeDef::required("email", AttrType::CiString),
            AttributeDef::required("name", AttrType::String),
        ];
        static CONTACT: ResourceDef = ResourceDef {
            name: "Contact",
            table: "contacts",
            attributes: ATTRS,
            relationships: &[],
            actions: &[],
            policies: &[],
            field_policies: &[],
            calculations: &[],
            aggregates: &[],
            extensions: &[],
            notifiers: &[],
            identities: &[],
            indexes: &[],
            checks: &[],
            statements: &[],
            embedded: false,
            data_layer: crate::DataLayerKind::Memory,
            timestamps: None,
            store_type_id: || std::any::TypeId::of::<()>(),
            store_name: "memory",
            multitenancy: None,
        };
        let row = fields! { "email" => "Ada@Example.com", "name" => "Ada" };

        let email = Filter::ends_with("email", "EXAMPLE.COM");
        assert!(!email.matches(&row));
        assert!(email.matches_on(&CONTACT, &row));
        assert!(!Filter::contains("name", "ADA").matches_on(&CONTACT, &row));
        assert!((!Filter::contains("name", "ADA")).matches_on(&CONTACT, &row));
    }

   
    #[test]
    fn operator_overloading() {
        let open = Filter::eq("status", "open");
        let active = Filter::eq("active", true);
        let closed = Filter::eq("status", "closed");

        let and_filter = open.clone() & active.clone();
        assert_eq!(and_filter, Filter::and([open.clone(), active.clone()]));

        let or_filter = open.clone() | closed.clone();
        assert_eq!(or_filter, Filter::or([open.clone(), closed.clone()]));

        let complex = (open & active) | closed;
        assert!(matches!(complex, Filter::Or(_)));
    }
}
