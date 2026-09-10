pub mod ast;
pub mod codegen;
pub mod parse;

pub use ast::DomainDefinition;
pub use codegen::expand_domain;

pub fn expand_dsl(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let extracted = crate::ast_helpers::extract_domain_ident(&input);
    match syn::parse2::<DomainDefinition>(input) {
        Ok(def) => {
            let name = def.domain_name.clone();
            match expand_domain(def) {
                Ok(tokens) => tokens,
                Err(err) => error_with_fallback_stub(err, Some(&name)),
            }
        }
        Err(err) => error_with_fallback_stub(err, extracted.as_ref()),
    }
}

fn error_with_fallback_stub(
    err: syn::Error,
    domain: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    let compile_error = err.to_compile_error();
    match domain {
        Some(domain_name) => {
            let stub = fallback_domain_stub(domain_name);
            quote::quote! {
                #compile_error
                #stub
            }
        }
        None => compile_error,
    }
}

pub fn fallback_domain_stub(domain_name: &syn::Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        #[doc(hidden)]
        #[allow(dead_code)]
        pub struct #domain_name<D: ::ash_core::DataLayer> {
            pub ctx: ::ash_core::Context<D>,
        }
        impl<D: ::ash_core::DataLayer> #domain_name<D> {
            pub fn new(ctx: ::ash_core::Context<D>) -> Self {
                Self { ctx }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_parse_error_emits_fallback_stub() {
        let tokens = quote! {
            domain Broken;
            resourcez {
                Ticket
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "missing compile_error: {out}"
        );
        assert!(out.contains("struct Broken"), "missing stub struct: {out}");
        assert!(out.contains("Did you mean"), "missing suggestion: {out}");
    }
}
