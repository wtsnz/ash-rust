use std::fmt;
use uuid::Uuid;

#[derive(Debug)]
pub enum Error {
    Forbidden,
    NotFound,
    TooMany(usize),
    UnknownAction {
        resource: &'static str,
        name: String,
    },
    WrongActionKind {
        action: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
    NotAccepted {
        field: String,
        action: &'static str,
    },
    Missing {
        field: String,
    },
    TypeMismatch {
        field: String,
        expected: String,
        got: String,
    },
    Constraint {
        field: String,
        message: String,
    },
    /// A value an action's validations refuse, as Ash's `InvalidAttribute`: `message`
    /// is a template, its `%{var}`s filled from `vars` (see [`Error::message`]), as Ash
    /// writes a validation's message.
    Validation {
        field: String,
        message: String,
        vars: Vec<(String, crate::value::Value)>,
    },
    NoPrimaryKey(&'static str),
    NoPrimaryRead(&'static str),
    ManualRequired {
        action: &'static str,
    },
    NotManual {
        action: &'static str,
    },
    DataLayer(String),
    Invalid(String),
    Extension(Box<dyn std::error::Error + Send + Sync>),
    StaleRecord {
        resource: &'static str,
        id: Uuid,
    },
    /// An update that must run atomically can't, as Ash's `MustBeAtomic`.
    MustBeAtomic {
        resource: &'static str,
        action: &'static str,
        reason: String,
    },
    IdentityConflict {
        identity: &'static str,
        fields: Vec<String>,
        message: String,
    },
    Multi {
        step: String,
        source: Box<Error>,
    },
    DeleteRestricted {
        resource: &'static str,
        relationship: &'static str,
        count: usize,
    },
    TenantRequired {
        resource: &'static str,
    },
    Authentication(String),
    /// Several errors at once, as Ash reports every validation a changeset fails.
    Multiple(Vec<Error>),
}

/// `template` with each `%{var}` replaced by its value in `vars`.
pub fn interpolate(template: &str, vars: &[(String, crate::value::Value)]) -> String {
    let mut out = template.to_string();
    for (name, value) in vars {
        let text = match value {
            crate::value::Value::Null => "nil".to_string(),
            crate::value::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        out = out.replace(&format!("%{{{name}}}"), &text);
    }
    out
}

impl Error {
    /// A validation's error: `message` a template, its `%{var}`s filled from `vars`.
    pub fn validation(field: impl Into<String>, message: impl Into<String>, vars: Vec<(String, crate::value::Value)>) -> Self {
        Self::Validation { field: field.into(), message: message.into(), vars }
    }

    /// `errors` as one error: none at all, one as itself, more as [`Error::Multiple`].
    pub fn collect(mut errors: Vec<Error>) -> std::result::Result<(), Error> {
        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.remove(0)),
            _ => Err(Self::Multiple(errors)),
        }
    }

    /// The errors this one holds, by value: its own several, or itself.
    pub fn into_each(self) -> Vec<Error> {
        match self {
            Self::Multiple(errors) => errors.into_iter().flat_map(Error::into_each).collect(),
            other => vec![other],
        }
    }

    /// Each error this one holds: its own several, or itself.
    pub fn each(&self) -> Vec<&Error> {
        match self {
            Self::Multiple(errors) => errors.iter().flat_map(Error::each).collect(),
            other => vec![other],
        }
    }

    /// What a person reads of the error: a validation's message with its vars filled in.
    pub fn message(&self) -> String {
        match self {
            Self::Validation { message, vars, .. } => interpolate(message, vars),
            Self::Constraint { message, .. } => message.clone(),
            other => other.to_string(),
        }
    }

    pub fn multi_step(&self) -> Option<&str> {
        match self {
            Self::Multi { step, .. } => Some(step.as_str()),
            _ => None,
        }
    }

    pub fn multi_source(&self) -> Option<&Error> {
        match self {
            Self::Multi { source, .. } => Some(source.as_ref()),
            _ => None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Forbidden => write!(f, "forbidden"),
            Self::NotFound => write!(f, "not found"),
            Self::TooMany(n) => write!(f, "expected one record, found {n}"),
            Self::UnknownAction { resource, name } => {
                write!(f, "unknown action `{name}` on {resource}")
            }
            Self::WrongActionKind {
                action,
                expected,
                actual,
            } => write!(f, "action `{action}` is {actual}, expected {expected}"),
            Self::NotAccepted { field, action } => {
                write!(f, "field `{field}` is not accepted by action `{action}`")
            }
            Self::Missing { field } => write!(f, "missing required attribute `{field}`"),
            Self::TypeMismatch {
                field,
                expected,
                got,
            } => write!(f, "attribute `{field}` expected {expected}, got {got}"),
            Self::Constraint { field, message } => {
                write!(f, "attribute `{field}` {message}")
            }
            Self::Validation { field, message, vars } => {
                write!(f, "validation failed on `{field}`: {}", interpolate(message, vars))
            }
            Self::NoPrimaryKey(resource) => write!(f, "resource {resource} has no primary key"),
            Self::NoPrimaryRead(resource) => {
                write!(f, "resource {resource} has no primary read action")
            }
            Self::ManualRequired { action } => {
                write!(f, "action `{action}` is manual; use manual_create")
            }
            Self::NotManual { action } => {
                write!(f, "action `{action}` persists through the data layer")
            }
            Self::DataLayer(message) => write!(f, "{message}"),
            Self::Invalid(message) => write!(f, "{message}"),
            Self::Extension(err) => write!(f, "{err}"),
            Self::MustBeAtomic { resource, action, reason } => write!(
                f,
                "{resource}.{action} must be performed atomically, but it could not be: {reason}; \
                 set `require_atomic` to false on the action to read the record first instead"
            ),
            Self::StaleRecord { resource, id } => {
                write!(
                    f,
                    "stale record on {resource} `{id}`: record was modified concurrently"
                )
            }
            Self::IdentityConflict {
                identity,
                fields,
                message,
            } => {
                write!(
                    f,
                    "identity conflict on `{identity}` ({:?}): {message}",
                    fields
                )
            }
            Self::Multi { step, source } => {
                write!(f, "multi step `{step}` failed: {source}")
            }
            Self::DeleteRestricted {
                resource,
                relationship,
                count,
            } => {
                write!(
                    f,
                    "cannot delete {resource} because relationship `{relationship}` has {count} dependent record(s) and specifies on_delete: restrict"
                )
            }
            Self::TenantRequired { resource } => {
                write!(f, "tenant is required for resource `{resource}`")
            }
            Self::Authentication(msg) => write!(f, "authentication error: {msg}"),
            Self::Multiple(errors) => {
                let messages: Vec<String> = errors.iter().map(ToString::to_string).collect();
                write!(f, "{}", messages.join("; "))
            }
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
