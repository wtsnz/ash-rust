use crate::define::ast::{PolicyCheckExpr, PolicySpec, PolicyWhenSpec};
use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Lit, Result};

pub fn lit_to_const_value(lit: &Lit) -> Result<TokenStream> {
    match lit {
        Lit::Str(s) => Ok(quote! { ::ash_core::ConstValue::Str(#s) }),
        Lit::Bool(b) => Ok(quote! { ::ash_core::ConstValue::Bool(#b) }),
        Lit::Int(i) => {
            let val = i.base10_parse::<i64>()?;
            Ok(quote! { ::ash_core::ConstValue::Int(#val) })
        }
        other => Err(Error::new_spanned(
            other,
            "unsupported literal for ConstValue; expected string, bool, or integer",
        )),
    }
}

pub fn check_to_tokens(check: &PolicyCheckExpr) -> Result<TokenStream> {
    match check {
        PolicyCheckExpr::Always => Ok(quote! { ::ash_core::Check::Always }),
        PolicyCheckExpr::ActorPresent => Ok(quote! { ::ash_core::Check::ActorPresent }),
        PolicyCheckExpr::RelatesToActor(f) => {
            Ok(quote! { ::ash_core::Check::RelatesToActor { field: #f } })
        }
        PolicyCheckExpr::IsNil(f) => Ok(quote! { ::ash_core::Check::IsNil { field: #f } }),
        PolicyCheckExpr::ActorAttributeEquals { attr, value } => {
            let val_tok = lit_to_const_value(value)?;
            Ok(quote! { ::ash_core::Check::ActorAttributeEquals { attr: #attr, value: #val_tok } })
        }
        PolicyCheckExpr::Eq { field, value } => {
            let val_tok = lit_to_const_value(value)?;
            Ok(quote! { ::ash_core::Check::Eq { field: #field, value: #val_tok } })
        }
        PolicyCheckExpr::And(parts) => {
            let tokens = parts
                .iter()
                .map(check_to_tokens)
                .collect::<Result<Vec<_>>>()?;
            Ok(quote! { ::ash_core::Check::And(&[#(#tokens),*]) })
        }
        PolicyCheckExpr::Or(parts) => {
            let tokens = parts
                .iter()
                .map(check_to_tokens)
                .collect::<Result<Vec<_>>>()?;
            Ok(quote! { ::ash_core::Check::Or(&[#(#tokens),*]) })
        }
    }
}

pub fn expand_policy_defs(policies: &[PolicySpec]) -> Result<Vec<TokenStream>> {
    let mut policy_defs = Vec::new();
    for pol in policies {
        let check_tokens = pol
            .checks
            .iter()
            .map(|ch| {
                let tok = check_to_tokens(ch)?;
                Ok(quote! { ::ash_core::PolicyEffect::AuthorizeIf(#tok) })
            })
            .collect::<Result<Vec<_>>>()?;

        for when in &pol.whens {
            let when_tok = match when {
                PolicyWhenSpec::Always => quote! { ::ash_core::PolicyWhen::Always },
                PolicyWhenSpec::ActionName(n) => quote! { ::ash_core::PolicyWhen::ActionName(#n) },
                PolicyWhenSpec::ActionKind(k) => {
                    let variant = k.kind_variant();
                    quote! { ::ash_core::PolicyWhen::ActionType(::ash_core::ActionKind::#variant) }
                }
            };
            policy_defs.push(quote! {
                ::ash_core::PolicyDef::when(#when_tok, &[#(#check_tokens),*])
            });
        }
    }
    Ok(policy_defs)
}

pub fn expand_field_policy_defs(
    fps: &[crate::define::ast::FieldPolicySpec],
) -> Result<Vec<TokenStream>> {
    let mut fp_defs = Vec::new();
    for fp in fps {
        let field_str = fp.field.to_string();
        let check_tokens = fp
            .checks
            .iter()
            .map(|ch| {
                let tok = check_to_tokens(ch)?;
                Ok(quote! { ::ash_core::PolicyEffect::AuthorizeIf(#tok) })
            })
            .collect::<Result<Vec<_>>>()?;
        fp_defs.push(quote! {
            ::ash_core::FieldPolicyDef::new(#field_str, &[#(#check_tokens),*])
        });
    }
    Ok(fp_defs)
}
