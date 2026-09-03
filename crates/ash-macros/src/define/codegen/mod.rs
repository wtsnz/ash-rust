pub mod actions;
pub mod calculations;
pub mod policies;
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
