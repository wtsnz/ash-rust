pub mod actions;
pub mod calculations;
pub mod policies;
pub mod probe;
pub mod resource;

use crate::define::ast::ResourceDefinition;
use proc_macro2::TokenStream;
use quote::quote;
use syn::Result;

pub fn expand_define(mut def: ResourceDefinition) -> Result<TokenStream> {
    for action in &mut def.actions {
        if action.kind != crate::define::ast::ActionKind::Generic {
            for acc in &mut action.accept {
                if acc.inferred {
                    if let Some(attr) = def.attributes.iter().find(|a| a.ident == acc.name) {
                        acc.ty = attr.ty.clone();
                    }
                }
            }
        }
    }

    let resource = &def.resource;
    let struct_and_resource_tokens = resource::expand_resource_struct(&def)?;
    let (action_defs, has_primary_read) = actions::expand_action_defs(&def)?;
    let policy_defs = policies::expand_policy_defs(&def.policies)?;
    let field_policy_defs = policies::expand_field_policy_defs(&def.field_policies)?;
    let action_codegen = actions::expand_action_builders(&def, has_primary_read);
    let probe_tokens = probe::expand_ide_probe(&def);
    let warnings = &def.warnings;

    let builders = action_codegen.builders;
    let resource_methods = action_codegen.resource_methods;
    let trait_block = action_codegen.trait_block;

    let extend_calls: Vec<TokenStream> = def
        .extends
        .iter()
        .map(|ext| {
            let macro_path = &ext.macro_path;
            let tokens = &ext.tokens;
            quote! {
                #macro_path!(#resource, { #tokens });
            }
        })
        .collect();

    Ok(quote! {
        #(#warnings)*

        #probe_tokens

        #struct_and_resource_tokens

        impl #resource {
            pub const ACTIONS: &'static [::ash_core::ActionDef] = &[
                #(#action_defs),*
            ];

            pub const POLICIES: &'static [::ash_core::PolicyDef] = &[
                #(#policy_defs),*
            ];

            pub const FIELD_POLICIES: &'static [::ash_core::FieldPolicyDef] = &[
                #(#field_policy_defs),*
            ];

            #(#resource_methods)*
        }

        #(#builders)*

        #trait_block

        #(#extend_calls)*
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn parse_def(tokens: proc_macro2::TokenStream) -> ResourceDefinition {
        match syn::parse2::<ResourceDefinition>(tokens) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {e}"),
        }
    }

    fn expand_ok(tokens: proc_macro2::TokenStream) -> TokenStream {
        let mut def = parse_def(tokens);
        let errors = crate::define::validate::validate(&mut def);
        assert!(
            errors.is_empty(),
            "unexpected validation errors: {errors:?}"
        );
        expand_define(def).expect("expand")
    }

    #[test]
    fn test_belongs_to_matching_fk_expands() {
        let _ = expand_ok(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    author_id: Uuid;
                }
                relationships {
                    belongs_to author: User [fk: author_id];
                }
                actions {
                    read read { primary; }
                }
            }
        });
    }

    #[test]
    fn test_uncovered_action_is_a_validation_error() {
        let mut def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    create open { primary; accept [title]; }
                    read read { primary; }
                    update assign { accept [title]; }
                }
                policies {
                    policy action(open) {
                        authorize_if always;
                    }
                    policy action_type(read) {
                        authorize_if always;
                    }
                }
            }
        });
        let errors = crate::define::validate::validate(&mut def);
        let msg = errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains(
                "Action 'assign' has no matching policy rule and will always be forbidden at runtime"
            ),
            "missing uncovered action error: {msg}"
        );
        assert!(
            !msg.contains("Action 'open' has no matching policy rule"),
            "open should be covered: {msg}"
        );
        assert!(
            !msg.contains("Action 'read' has no matching policy rule"),
            "read should be covered: {msg}"
        );
    }

    #[test]
    fn test_policy_always_covers_all_actions() {
        let out = expand_ok(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                }
                actions {
                    create open { primary; }
                    read read { primary; }
                }
                policies {
                    policy always {
                        authorize_if always;
                    }
                }
            }
        })
        .to_string();
        assert!(
            !out.contains("has no matching policy rule"),
            "always should cover all actions: {out}"
        );
    }

    #[test]
    fn test_typestate_emits_named_missing_input_diagnostic() {
        let out = expand_ok(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    create open {
                        primary;
                        accept [title];
                    }
                    read read { primary; }
                }
            }
        })
        .to_string();
        assert!(
            out.contains("missing `.title(...)` on this action builder"),
            "missing typestate diagnostic: {out}"
        );
    }

    #[test]
    fn test_attribute_docs_copied_to_field_consts_and_setters() {
        let out = expand_ok(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    /// The ticket subject
                    subject: String;
                }
                actions {
                    create open {
                        accept [subject];
                        /// Reason supplied by the caller
                        argument reason: String;
                    }
                }
            }
        })
        .to_string();
        assert!(
            out.contains("The ticket subject"),
            "missing attribute docs: {out}"
        );
        assert!(
            out.contains("Reason supplied by the caller"),
            "missing argument docs: {out}"
        );
    }
}
