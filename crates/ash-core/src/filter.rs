use std::cmp::Ordering;

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
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
}

impl Filter {
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
            | Self::IsNil(field) => out.push(field),
            Self::And(parts) | Self::Or(parts) => {
                for part in parts {
                    part.collect_fields(out);
                }
            }
            Self::Not(inner) => inner.collect_fields(out),
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

    pub fn matches(&self, fields: &FieldMap) -> bool {
        match self {
            Self::True => true,
            Self::False => false,
            Self::Eq(field, value) => {
                if value.is_null() {
                    matches!(fields.get(field), None | Some(Value::Null))
                } else {
                    fields.get(field).is_some_and(|got| !got.is_null() && got == value)
                }
            }
            Self::Ne(field, value) => {
                if value.is_null() {
                    fields.get(field).is_some_and(|got| !got.is_null())
                } else {
                    fields.get(field).is_some_and(|got| !got.is_null() && got != value)
                }
            }
            Self::Gt(field, value) => compare(fields.get(field), value, Ordering::Greater, false),
            Self::Gte(field, value) => compare(fields.get(field), value, Ordering::Greater, true),
            Self::Lt(field, value) => compare(fields.get(field), value, Ordering::Less, false),
            Self::Lte(field, value) => compare(fields.get(field), value, Ordering::Less, true),
            Self::In(field, values) => fields.get(field).is_some_and(|got| values.contains(got)),
            Self::IsNil(field) => matches!(fields.get(field), None | Some(Value::Null)),
            Self::And(parts) => parts.iter().all(|part| part.matches(fields)),
            Self::Or(parts) => parts.iter().any(|part| part.matches(fields)),
            Self::Not(inner) => !inner.matches(fields),
        }
    }
}

fn compare(got: Option<&Value>, rhs: &Value, direction: Ordering, equal_ok: bool) -> bool {
    let Some(got) = got else {
        return false;
    };
    if got.is_null() || rhs.is_null() {
        return false;
    }
    match got.cmp(rhs) {
        Ordering::Equal => equal_ok,
        order => order == direction,
    }
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
