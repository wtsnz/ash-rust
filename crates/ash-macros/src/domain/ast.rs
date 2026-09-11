use syn::{Ident, Type};

pub struct DomainDefinition {
    pub outer_attrs: Vec<syn::Attribute>,
    pub domain_name: Ident,
    pub resources: Vec<DomainResourceSpec>,
}

impl DomainDefinition {
    pub fn empty(domain_name: Ident) -> Self {
        Self {
            outer_attrs: Vec::new(),
            domain_name,
            resources: Vec::new(),
        }
    }
}

pub struct DomainResourceSpec {
    pub resource: Ident,
    pub interfaces: Vec<CodeInterfaceSpec>,
}

pub struct CodeInterfaceSpec {
    pub fn_name: Ident,
    pub action_name: Ident,
    pub args: Vec<CodeInterfaceArg>,
    pub get_by: Option<Ident>,
    pub target: CodeInterfaceTarget,
}

pub struct CodeInterfaceArg {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum CodeInterfaceTarget {
    #[default]
    Static,
    Record,
    Id,
}
