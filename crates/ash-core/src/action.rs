use crate::actor::Actor;
use crate::error::Result;
use crate::resource::AttrType;
use crate::value::{ConstValue, FieldMap};

pub struct ChangeContext<'a> {
    pub fields: &'a mut FieldMap,
    pub actor: Option<&'a Actor>,
    pub arguments: &'a FieldMap,
}

pub trait CustomChange: Send + Sync + 'static {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()>;
}

pub struct ValidationContext<'a> {
    pub record: Option<&'a FieldMap>,
    pub fields: &'a FieldMap,
    pub arguments: &'a FieldMap,
}

pub trait CustomValidation: Send + Sync + 'static {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()>;
}

#[derive(Clone, Copy, Debug)]
pub struct ArgumentDef {
    pub name: &'static str,
    pub ty: AttrType,
    pub allow_nil: bool,
}

impl ArgumentDef {
    pub const fn new(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            allow_nil: false,
        }
    }

    pub const fn optional(name: &'static str, ty: AttrType) -> Self {
        Self {
            name,
            ty,
            allow_nil: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Create,
    Read,
    Update,
    Destroy,
    Generic,
}

impl ActionKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Read => "read",
            Self::Update => "update",
            Self::Destroy => "destroy",
            Self::Generic => "generic",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PersistKind {
    DataLayer,
    Manual,
}

#[derive(Clone, Copy)]
pub enum PreparationDef {
    Filter(fn() -> crate::filter::Filter),
    Sort { field: &'static str, descending: bool },
    Limit(usize),
    Offset(usize),
}

impl std::fmt::Debug for PreparationDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Filter(_) => f.write_str("PreparationDef::Filter(..)"),
            Self::Sort { field, descending } => f
                .debug_struct("Sort")
                .field("field", field)
                .field("descending", descending)
                .finish(),
            Self::Limit(n) => f.debug_tuple("Limit").field(n).finish(),
            Self::Offset(n) => f.debug_tuple("Offset").field(n).finish(),
        }
    }
}

impl PreparationDef {
    pub const fn filter(f: fn() -> crate::filter::Filter) -> Self {
        Self::Filter(f)
    }

    pub const fn sort(field: &'static str, descending: bool) -> Self {
        Self::Sort { field, descending }
    }

    pub const fn limit(limit: usize) -> Self {
        Self::Limit(limit)
    }

    pub const fn offset(offset: usize) -> Self {
        Self::Offset(offset)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ActionDef {
    pub name: &'static str,
    pub kind: ActionKind,
    pub accept: &'static [&'static str],
    pub arguments: &'static [ArgumentDef],
    pub primary: bool,
    pub changes: &'static [Change],
    pub validations: &'static [Validation],
    pub preparations: &'static [PreparationDef],
    pub persist: PersistKind,
}

impl ActionDef {
    pub const fn create(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Create,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
        }
    }

    pub const fn read(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Read,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
        }
    }

    pub const fn update(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Update,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
        }
    }

    pub const fn destroy(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Destroy,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
        }
    }

    pub const fn generic(name: &'static str) -> Self {
        Self {
            name,
            kind: ActionKind::Generic,
            accept: &[],
            arguments: &[],
            primary: false,
            changes: &[],
            validations: &[],
            preparations: &[],
            persist: PersistKind::DataLayer,
        }
    }

    pub const fn accept(mut self, accept: &'static [&'static str]) -> Self {
        self.accept = accept;
        self
    }

    pub const fn arguments(mut self, arguments: &'static [ArgumentDef]) -> Self {
        self.arguments = arguments;
        self
    }

    pub fn argument(&self, name: &str) -> Option<&ArgumentDef> {
        self.arguments.iter().find(|arg| arg.name == name)
    }

    pub fn has_argument(&self, name: &str) -> bool {
        self.argument(name).is_some()
    }

    pub const fn changes(mut self, changes: &'static [Change]) -> Self {
        self.changes = changes;
        self
    }

    pub const fn validations(mut self, validations: &'static [Validation]) -> Self {
        self.validations = validations;
        self
    }

    pub const fn preparations(mut self, preparations: &'static [PreparationDef]) -> Self {
        self.preparations = preparations;
        self
    }

    pub const fn primary(mut self) -> Self {
        self.primary = true;
        self
    }

    pub const fn manual(mut self) -> Self {
        self.persist = PersistKind::Manual;
        self
    }
}

#[derive(Clone, Copy)]
pub enum Change {
    SetAttribute {
        field: &'static str,
        value: ConstValue,
    },
    SetNewAttribute {
        field: &'static str,
        value: ConstValue,
    },
    RelateActor {
        field: &'static str,
    },
    SetFromArgument {
        field: &'static str,
        argument: &'static str,
    },
    Custom(&'static dyn CustomChange),
    Func(fn(&mut ChangeContext<'_>) -> Result<()>),
}

impl std::fmt::Debug for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SetAttribute { field, value } => f
                .debug_struct("SetAttribute")
                .field("field", field)
                .field("value", value)
                .finish(),
            Self::SetNewAttribute { field, value } => f
                .debug_struct("SetNewAttribute")
                .field("field", field)
                .field("value", value)
                .finish(),
            Self::RelateActor { field } => {
                f.debug_struct("RelateActor").field("field", field).finish()
            }
            Self::SetFromArgument { field, argument } => f
                .debug_struct("SetFromArgument")
                .field("field", field)
                .field("argument", argument)
                .finish(),
            Self::Custom(_) => write!(f, "Custom(<dyn CustomChange>)"),
            Self::Func(_) => write!(f, "Func(<fn>)"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Validation {
    Present {
        field: &'static str,
    },
    StringLength {
        field: &'static str,
        min: Option<usize>,
        max: Option<usize>,
    },
    OneOf {
        field: &'static str,
        allowed: &'static [&'static str],
    },
    Numericality {
        field: &'static str,
        min: Option<i64>,
        max: Option<i64>,
    },
    Custom(&'static dyn CustomValidation),
    Func(fn(&ValidationContext<'_>) -> Result<()>),
}

impl std::fmt::Debug for Validation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Present { field } => {
                f.debug_struct("Present").field("field", field).finish()
            }
            Self::StringLength { field, min, max } => f
                .debug_struct("StringLength")
                .field("field", field)
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::OneOf { field, allowed } => f
                .debug_struct("OneOf")
                .field("field", field)
                .field("allowed", allowed)
                .finish(),
            Self::Numericality { field, min, max } => f
                .debug_struct("Numericality")
                .field("field", field)
                .field("min", min)
                .field("max", max)
                .finish(),
            Self::Custom(_) => write!(f, "Custom(<dyn CustomValidation>)"),
            Self::Func(_) => write!(f, "Func(<fn>)"),
        }
    }
}

impl Validation {
    pub const fn present(field: &'static str) -> Self {
        Self::Present { field }
    }

    pub const fn string_length(
        field: &'static str,
        min: Option<usize>,
        max: Option<usize>,
    ) -> Self {
        Self::StringLength { field, min, max }
    }

    pub const fn one_of(field: &'static str, allowed: &'static [&'static str]) -> Self {
        Self::OneOf { field, allowed }
    }

    pub const fn numericality(
        field: &'static str,
        min: Option<i64>,
        max: Option<i64>,
    ) -> Self {
        Self::Numericality { field, min, max }
    }

    pub const fn custom(c: &'static dyn CustomValidation) -> Self {
        Self::Custom(c)
    }

    pub const fn func(f: fn(&ValidationContext<'_>) -> Result<()>) -> Self {
        Self::Func(f)
    }
}

impl Change {
    pub const fn custom(c: &'static dyn CustomChange) -> Self {
        Self::Custom(c)
    }

    pub const fn func(f: fn(&mut ChangeContext<'_>) -> Result<()>) -> Self {
        Self::Func(f)
    }
}
