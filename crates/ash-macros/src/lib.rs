use proc_macro::TokenStream;
use syn::{DeriveInput, parse_macro_input};

mod ast_helpers;
mod define;
mod derive;
mod domain;

#[proc_macro_derive(Resource, attributes(ash))]
pub fn derive_resource(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive::expand(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro]
pub fn domain(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as domain::DomainDefinition);
    match domain::expand_domain(def) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro]
pub fn resource(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as define::ResourceDefinition);
    match define::expand_define(def) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.into_compile_error().into(),
    }
}

#[proc_macro]
pub fn define(input: TokenStream) -> TokenStream {
    let def = parse_macro_input!(input as define::ResourceDefinition);
    match define::expand_define(def) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.into_compile_error().into(),
    }
}
