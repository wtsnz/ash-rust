pub mod ast;
mod codegen;
mod parse;
mod validate;

use crate::ast_helpers::combine_errors;
pub use ast::ResourceDefinition;

pub fn expand_dsl(input: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let extracted = crate::ast_helpers::extract_resource_ident(&input);
    let mut parsed = parse::parse_resource(input);
    parsed.errors.extend(validate::validate(&mut parsed.def));

    let stub = if parsed.def.attributes.is_empty() {
        extracted
            .as_ref()
            .map(fallback_resource_stub)
            .unwrap_or_else(|| richer_fallback_resource_stub(&parsed.def))
    } else {
        richer_fallback_resource_stub(&parsed.def)
    };

    match codegen::expand_define(parsed.def) {
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

fn richer_fallback_resource_stub(def: &ResourceDefinition) -> proc_macro2::TokenStream {
    let resource = &def.resource;
    let attr_names: Vec<_> = def.attributes.iter().map(|a| &a.ident).collect();
    let attr_tys: Vec<_> = def.attributes.iter().map(|a| &a.ty).collect();
    let calc_names: Vec<_> = def.calculations.iter().map(|c| &c.ident).collect();
    let calc_tys: Vec<_> = def.calculations.iter().map(|c| &c.ty).collect();

    let id_body = def
        .attributes
        .iter()
        .find(|a| a.pk && crate::ast_helpers::is_uuid(&a.ty))
        .map(|a| {
            let id = &a.ident;
            quote::quote! { self.#id }
        })
        .unwrap_or_else(|| quote::quote! { ::uuid::Uuid::nil() });

    let fields = if attr_names.is_empty() && calc_names.is_empty() {
        quote::quote! {
            pub id: ::uuid::Uuid,
        }
    } else {
        quote::quote! {
            #(pub #attr_names: #attr_tys,)*
            #(pub #calc_names: #calc_tys,)*
        }
    };

    quote::quote! {
        #[doc(hidden)]
        #[allow(dead_code)]
        #[derive(Clone, Debug)]
        pub struct #resource {
            #fields
        }

        #[doc(hidden)]
        #[allow(dead_code, unused_variables)]
        impl ::ash_core::Resource for #resource {
            type Store = ::ash_core::DefaultStore;

            const DEF: ::ash_core::ResourceDef = ::ash_core::ResourceDef {
                name: stringify!(#resource),
                table: "",
                attributes: &[],
                relationships: &[],
                actions: &[],
                policies: &[],
                field_policies: &[],
                calculations: &[],
                aggregates: &[],
                extensions: &[],
                notifiers: &[],
                identities: &[],
                embedded: false,
                data_layer: ::ash_core::DataLayerKind::Memory,
                timestamps: None,
                store_type_id: ::ash_core::default_store_type_id,
                store_name: "DefaultStore",
                multitenancy: None,
            };

            fn id(&self) -> ::uuid::Uuid {
                #id_body
            }

            fn to_fields(&self) -> ::ash_core::FieldMap {
                ::ash_core::FieldMap::new()
            }

            fn from_fields(
                _: &::ash_core::FieldMap,
            ) -> ::ash_core::Result<Self> {
                Err(::ash_core::Error::Invalid(
                    ::std::string::String::from("resource expansion failed"),
                ))
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
        assert!(out.contains("subject"), "missing parsed attribute: {out}");
        assert!(
            out.contains("impl") && out.contains("Resource"),
            "missing Resource stub impl: {out}"
        );
    }

    #[test]
    fn test_recovery_keeps_good_actions_and_reports_bad_one() {
        let tokens = quote! {
            resource Ticket;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                creat broken;
                create open {
                    accept [subject];
                }
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(
            out.contains("compile_error"),
            "missing compile_error: {out}"
        );
        assert!(out.contains("Did you mean"), "missing suggestion: {out}");
        assert!(out.contains("struct Ticket"), "missing struct: {out}");
        assert!(
            out.contains("open") || out.contains("Open"),
            "missing recovered open action: {out}"
        );
    }

    #[test]
    fn test_multiple_semantic_errors_are_combined() {
        let tokens = quote! {
            resource Ticket;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                create open {
                    accept [subjet];
                    change set(statu = "open");
                }
            }
        };
        let out = expand_dsl(tokens).to_string();
        assert!(out.contains("subjet"), "missing subjet error: {out}");
        assert!(out.contains("statu"), "missing statu error: {out}");
        assert!(out.contains("struct Ticket"), "missing struct: {out}");
    }
}
