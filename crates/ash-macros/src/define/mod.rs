pub mod ast;
mod codegen;
mod parse;

pub use ast::ResourceDefinition;
pub use codegen::expand_define;
