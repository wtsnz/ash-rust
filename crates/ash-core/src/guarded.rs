//! A field a field policy may hide, as a typed record holds it.

/// A typed record's field a field policy may hide from the actor reading it: its value,
/// or forbidden, as Ash puts `%Ash.ForbiddenField{}` in a hidden field's place. A record
/// can hold a hidden field whatever its type, an `Option` or not.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Guarded<T> {
    Value(T),
    /// Hidden from the actor that read the record.
    Forbidden,
}

impl<T> Guarded<T> {
    /// The value, unless it's hidden.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Forbidden => None,
        }
    }

    /// The value, unless it's hidden.
    pub fn into_value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            Self::Forbidden => None,
        }
    }

    pub fn is_forbidden(&self) -> bool {
        matches!(self, Self::Forbidden)
    }

    /// The value; panics if it's hidden.
    #[track_caller]
    pub fn unwrap(self) -> T {
        self.into_value().expect("a field hidden by a field policy")
    }
}

impl<T> From<T> for Guarded<T> {
    fn from(value: T) -> Self {
        Self::Value(value)
    }
}

impl<T: PartialEq> PartialEq<T> for Guarded<T> {
    fn eq(&self, other: &T) -> bool {
        self.value() == Some(other)
    }
}

impl PartialEq<&str> for Guarded<String> {
    fn eq(&self, other: &&str) -> bool {
        self.value().is_some_and(|value| value == other)
    }
}
