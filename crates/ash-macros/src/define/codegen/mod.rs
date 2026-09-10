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
    // 0. Resolve accept field types from def.attributes for bracketed accept lists
    for action in &mut def.actions {
        for acc in &mut action.accept {
            if acc.inferred {
                let attr = def.attributes.iter().find(|a| a.ident == acc.name);
                if let Some(attr) = attr {
                    acc.ty = attr.ty.clone();
                } else {
                    let attr_names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
                    let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                    return Err(crate::ast_helpers::unknown_ident_error(&acc.name, &attr_refs, "attribute"));
                }
            }
        }

        for chg in &action.changes {
            match chg {
                crate::define::ast::ChangeSpec::SetFromArg { field, argument } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(field, &attr_refs, "attribute"));
                    }
                    if action.arguments.iter().all(|a| a.name != *argument) {
                        let arg_names: Vec<String> = action.arguments.iter().map(|a| a.name.to_string()).collect();
                        let arg_refs: Vec<&str> = arg_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(argument, &arg_refs, "argument"));
                    }
                }
                crate::define::ast::ChangeSpec::RelateActor { field } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(field, &attr_refs, "attribute"));
                    }
                }
                crate::define::ast::ChangeSpec::ManageRelationship { relationship, .. } => {
                    if def.relationships.iter().all(|r| r.ident != *relationship) {
                        let rel_names: Vec<String> = def.relationships.iter().map(|r| r.ident.to_string()).collect();
                        let rel_refs: Vec<&str> = rel_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(relationship, &rel_refs, "relationship"));
                    }
                }
                _ => {}
            }
        }

        for val in &action.validations {
            let field = match val {
                crate::define::ast::ValidationSpec::Present { field }
                | crate::define::ast::ValidationSpec::StringLength { field, .. }
                | crate::define::ast::ValidationSpec::OneOf { field, .. }
                | crate::define::ast::ValidationSpec::Numericality { field, .. } => Some(field),
                _ => None,
            };
            if let Some(field) = field {
                let is_attr = def.attributes.iter().any(|a| a.ident == *field);
                let is_arg = action.arguments.iter().any(|a| a.name == *field);
                if !is_attr && !is_arg {
                    let mut candidates: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
                    candidates.extend(action.arguments.iter().map(|a| a.name.to_string()));
                    let cand_refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
                    return Err(crate::ast_helpers::unknown_ident_error(field, &cand_refs, "attribute or argument"));
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
            Err(e) => panic!("parse failed: {}", e),
        }
    }

    fn expand_err(def: ResourceDefinition) -> syn::Error {
        match expand_define(def) {
            Err(e) => e,
            Ok(_) => panic!("expected expand_define error"),
        }
    }

    #[test]
    fn test_accept_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
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
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(err.to_string().contains("Did you mean `subject`?"), "got: {}", err);
    }

    #[test]
    fn test_validation_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                create open {
                    accept [subject];
                    validate present(subjet);
                }
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(err.to_string().contains("Did you mean `subject`?"), "got: {}", err);
    }

    #[test]
    fn test_set_from_arg_unknown_argument_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                reason: String,
            }
            actions {
                create open {
                    argument reason_input: String;
                    change set_from_arg(reason, reason_inpt);
                }
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(err.to_string().contains("Did you mean `reason_input`?"), "got: {}", err);
    }
}
