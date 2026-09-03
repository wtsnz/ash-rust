pub mod ast;
pub mod codegen;
pub mod parse;

pub use ast::DomainDefinition;
pub use codegen::expand_domain;
