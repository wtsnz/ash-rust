pub mod ast;
pub mod codegen;
pub mod parse;
mod validate;

pub use codegen::expand_domain;

use crate::ast_helpers::combine_errors;

pub fn expand_dsl(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let extracted = crate::ast_helpers::extract_domain_ident(&input);
    let mut parsed = parse::parse_domain(input);
    parsed.errors.extend(validate::validate(&parsed.def));

    let stub = fallback_domain_stub(extracted.as_ref().unwrap_or(&parsed.def.domain_name));

    match expand_domain(parsed.def) {
        Ok(tokens) => {
            if let Some(err) = combine_errors(parsed.errors) {
                let compile_error = err.to_compile_error();
                quote::quote! {
                    #compile_error
                    #tokens
                }
            } else {
                tokens
            }
        }
        Err(err) => {
            parsed.errors.push(err);
            let compile_error = combine_errors(parsed.errors)
                .map(|e| e.to_compile_error())
                .unwrap_or_default();
            quote::quote! {
                #compile_error
                #stub
            }
        }
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
            Broken {
                resourcez {
                    Ticket;
                }
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

    #[test]
    fn test_recovery_keeps_good_resources() {
        let tokens = quote! {
            Helpdesk {
                resources {
                    Ticket {
                        defin broken, action: open;
                        define open_ticket, action: open, args: [subject: String];
                    };
                    Representative;
                }
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "missing compile_error: {out}"
        );
        assert!(out.contains("Did you mean"), "missing suggestion: {out}");
        assert!(out.contains("struct Helpdesk"), "missing struct: {out}");
        assert!(out.contains("Ticket"), "missing Ticket: {out}");
        assert!(
            out.contains("Representative"),
            "missing Representative: {out}"
        );
        assert!(
            out.contains("open"),
            "missing recovered open interface: {out}"
        );
    }

    #[test]
    fn test_old_header_still_expands_with_error() {
        let tokens = quote! {
            domain Helpdesk;
            resources {
                Ticket;
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "missing compile_error: {out}"
        );
        assert!(
            out.contains("not `domain Name;`") || out.contains("domain Name"),
            "missing header error: {out}"
        );
        assert!(out.contains("struct Helpdesk"), "missing struct: {out}");
        assert!(out.contains("Ticket"), "missing recovered resource: {out}");
    }
}
