use quote::format_ident;
use syn::{Expr, Ident, Lit, Type};

pub struct ResourceDefinition {
    pub outer_attrs: Vec<syn::Attribute>,
    pub resource: Ident,
    pub table: Option<String>,
    pub attributes: Vec<AttributeSpec>,
    pub relationships: Vec<RelationshipSpec>,
    pub calculations: Vec<CalculationSpec>,
    pub aggregates: Vec<AggregateSpec>,
    pub actions: Vec<ActionSpec>,
    pub policies: Vec<PolicySpec>,
    pub field_policies: Vec<FieldPolicySpec>,
    pub extensions: Vec<Expr>,
    pub notifiers: Vec<Expr>,
    pub extends: Vec<ExtendSpec>,
    pub optimistic_lock: Option<Ident>,
    pub identities: Vec<IdentitySpec>,
    pub embedded: bool,
    pub data_layer: Option<Ident>,
    pub store: Option<Type>,
    pub timestamps: Option<TimestampsSpec>,
    pub multitenancy: Option<MultitenancySpec>,
    pub warnings: Vec<proc_macro2::TokenStream>,
}

#[derive(Clone, Debug)]
pub struct MultitenancySpec {
    pub attribute: Option<String>,
    pub strategy: Option<String>,
    pub global: bool,
}

#[derive(Clone, Debug)]
pub struct TimestampsSpec {
    pub created_at: Ident,
    pub updated_at: Ident,
}

pub struct IdentitySpec {
    pub name: Ident,
    pub keys: Vec<Ident>,
    pub message: Option<String>,
}

pub struct AttributeSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub ident: Ident,
    pub ty: Type,
    pub pk: bool,
    pub version: bool,
    pub generated: bool,
    pub atom: Option<Vec<String>>,
    pub is_enum: bool,
    pub default: Option<syn::Expr>,
    pub default_fn: Option<syn::Path>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RelType {
    BelongsTo,
    HasMany,
    HasOne,
    ManyToMany,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OnDeleteSpec {
    #[default]
    Nothing,
    Cascade,
    Nilify,
    Restrict,
}

pub struct RelationshipSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub kind: RelType,
    pub ident: Ident,
    pub dest: Ident,
    pub struct_field_ty: Type,
    pub fk: Option<Ident>,
    pub through: Option<Ident>,
    pub source_attribute_on_join_resource: Option<String>,
    pub destination_attribute_on_join_resource: Option<String>,
    pub on_delete: OnDeleteSpec,
}

pub struct CalculationSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub ident: Ident,
    pub arguments: Vec<ArgumentSpec>,
    pub ty: Type,
    pub expr: CalculationExprSpec,
}

#[derive(Clone, Debug)]
pub enum CalculationExprSpec {
    Arg(Ident),
    StringLength(Ident),
    Field(Ident),
    LitInt(i64),
    LitString(String),
    LitBool(bool),
    Null,
    Add(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Sub(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Mul(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Div(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Eq(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Ne(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Gt(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Gte(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Lt(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Lte(Box<CalculationExprSpec>, Box<CalculationExprSpec>),
    Concat(Vec<CalculationExprSpec>),
    Coalesce(Vec<CalculationExprSpec>),
    Lower(Box<CalculationExprSpec>),
    Upper(Box<CalculationExprSpec>),
    Length(Box<CalculationExprSpec>),
    IfElse {
        cond: Box<CalculationExprSpec>,
        then_expr: Box<CalculationExprSpec>,
        else_expr: Box<CalculationExprSpec>,
    },
    Custom(syn::Path),
}

pub struct AggregateSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub ident: Ident,
    pub ty: Type,
    pub relationship: Ident,
    pub kind: AggregateKindSpec,
    pub filter: Option<AggregateFilterSpec>,
}

#[derive(Clone, PartialEq, Eq)]
pub enum AggregateKindSpec {
    Count,
    Exists,
    First { field: Ident },
    Sum { field: Ident },
}

#[derive(Clone)]
pub enum AggregateFilterSpec {
    Eq { field: Ident, value: Lit },
    Ne { field: Ident, value: Lit },
}

#[derive(Clone)]
pub enum PreparationSpec {
    Filter { expr: Expr },
    Sort { field: Ident, descending: bool },
    Limit(usize),
    Offset(usize),
}

pub struct ActionSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub kind: ActionKind,
    pub name: Ident,
    pub primary: bool,
    pub accept: Vec<FieldAccept>,
    pub arguments: Vec<ArgumentSpec>,
    pub changes: Vec<ChangeSpec>,
    pub validations: Vec<ValidationSpec>,
    pub preparations: Vec<PreparationSpec>,
    pub persist_manual: bool,
    pub returns: Option<Type>,
    pub run_expr: Option<Expr>,
}

pub struct ArgumentSpec {
    pub outer_attrs: Vec<syn::Attribute>,
    pub name: Ident,
    pub ty: Type,
    pub allow_nil: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Create,
    Read,
    Update,
    Destroy,
    Generic,
}

impl ActionKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Read => "read",
            Self::Update => "update",
            Self::Destroy => "destroy",
            Self::Generic => "generic",
        }
    }

    pub fn method_ident(self) -> Ident {
        format_ident!("{}", self.as_str())
    }

    pub fn kind_variant(self) -> Ident {
        match self {
            Self::Create => format_ident!("Create"),
            Self::Read => format_ident!("Read"),
            Self::Update => format_ident!("Update"),
            Self::Destroy => format_ident!("Destroy"),
            Self::Generic => format_ident!("Generic"),
        }
    }
}

pub struct FieldAccept {
    pub name: Ident,
    pub ty: Type,
    pub inferred: bool,
}

pub enum ChangeSpec {
    Set {
        field: Ident,
        value: Lit,
    },
    SetNew {
        field: Ident,
        value: Lit,
    },
    RelateActor {
        field: Ident,
    },
    SetFromArg {
        field: Ident,
        argument: Ident,
    },
    ManageRelationship {
        relationship: Ident,
        rel_type: Ident,
    },
    BeforeAction(Expr),
    AfterAction(Expr),
    AfterTransaction(Expr),
    Custom(Expr),
    Func(Expr),
}

pub enum ValidationSpec {
    Present {
        field: Ident,
    },
    StringLength {
        field: Ident,
        min: Option<usize>,
        max: Option<usize>,
    },
    OneOf {
        field: Ident,
        allowed: Vec<String>,
    },
    Numericality {
        field: Ident,
        min: Option<i64>,
        max: Option<i64>,
    },
    Custom(Expr),
    Func(Expr),
}

pub struct PolicySpec {
    pub bypass: bool,
    pub whens: Vec<PolicyWhenSpec>,
    pub checks: Vec<PolicyEffectSpec>,
}

pub enum PolicyWhenSpec {
    Always,
    ActionName(Ident),
    ActionKind(ActionKind),
}

pub enum PolicyEffectSpec {
    AuthorizeIf(PolicyCheckExpr),
    AuthorizeUnless(PolicyCheckExpr),
    ForbidIf(PolicyCheckExpr),
    ForbidUnless(PolicyCheckExpr),
}

pub enum PolicyCheckExpr {
    Always,
    ActorPresent,
    RelatesToActor(Ident),
    IsNil(Ident),
    ActorAttributeEquals { attr: Ident, value: Lit },
    Eq { field: Ident, value: Lit },
    And(Vec<PolicyCheckExpr>),
    Or(Vec<PolicyCheckExpr>),
}

pub struct FieldPolicySpec {
    pub field: Ident,
    pub checks: Vec<PolicyEffectSpec>,
}

pub struct ExtendSpec {
    pub macro_path: syn::Path,
    pub tokens: proc_macro2::TokenStream,
}
