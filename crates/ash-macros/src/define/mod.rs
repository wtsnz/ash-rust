pub mod ast;
mod codegen;
mod parse;

pub use ast::ResourceDefinition;

pub fn expand_dsl(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let extracted = crate::ast_helpers::extract_resource_ident(&input);
    match syn::parse2::<ResourceDefinition>(input) {
        Ok(def) => {
            let name = def.resource.clone();
            match codegen::expand_define(def) {
                Ok(tokens) => tokens,
                Err(err) => error_with_fallback_stub(err, Some(&name)),
            }
        }
        Err(err) => error_with_fallback_stub(err, extracted.as_ref()),
    }
}

fn error_with_fallback_stub(
    err: syn::Error,
    resource: Option<&syn::Ident>,
) -> proc_macro2::TokenStream {
    let compile_error = err.to_compile_error();
    match resource {
        Some(resource) => {
            let stub = fallback_resource_stub(resource);
            quote::quote! {
                #compile_error
                #stub
            }
        }
        None => compile_error,
    }
}

pub fn fallback_resource_stub(resource: &syn::Ident) -> proc_macro2::TokenStream {
    quote::quote! {
        #[doc(hidden)]
        #[allow(dead_code)]
        #[derive(Clone, Debug)]
        pub struct #resource {
            pub id: ::uuid::Uuid,
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
            resource Broken;
            attributes {
                id: Uuid [pk],
            }
            actions {
                creat open;
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "missing compile_error: {out}"
        );
        assert!(out.contains("struct Broken"), "missing stub struct: {out}");
        assert!(out.contains("uuid"), "missing uuid field: {out}");
    }

    #[test]
    fn test_expand_error_emits_fallback_stub() {
        let tokens = quote! {
            resource Broken;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                create open {
                    accept [subjet];
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
        assert!(
            out.contains("Available attributes"),
            "missing candidate list: {out}"
        );
    }
}
