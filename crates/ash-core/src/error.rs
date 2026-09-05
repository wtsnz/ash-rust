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
    Validation {
        field: String,
        message: String,
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
}

impl Error {
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
            Self::Validation { field, message } => {
                write!(f, "validation failed on `{field}`: {message}")
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
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
