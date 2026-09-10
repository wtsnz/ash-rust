pub mod actions;
pub mod calculations;
pub mod policies;
pub mod probe;
pub mod resource;

use crate::ast_helpers::{
    find_closest_match, is_i64, is_integer, is_string, option_inner, unknown_ident_error,
};
use crate::define::ast::{
    ActionKind, CalculationExprSpec, PolicyCheckExpr, PolicyEffectSpec, PolicyWhenSpec, RelType,
    ResourceDefinition, ValidationSpec,
};
use proc_macro2::TokenStream;
use quote::quote;
use std::collections::HashSet;
use syn::{Error, Ident, Result, Type};

fn ident_refs(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}

fn unknown_field(ident: &Ident, names: &[String], kind: &str) -> Error {
    unknown_ident_error(ident, &ident_refs(names), kind)
}

fn attr_and_rel_names(def: &ResourceDefinition) -> Vec<String> {
    let mut names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
    names.extend(def.relationships.iter().map(|r| r.ident.to_string()));
    names
}

fn validate_policy_check(check: &PolicyCheckExpr, field_names: &[String]) -> Result<()> {
    match check {
        PolicyCheckExpr::RelatesToActor(field)
        | PolicyCheckExpr::IsNil(field)
        | PolicyCheckExpr::Eq { field, .. } => {
            if !field_names.iter().any(|n| n == &field.to_string()) {
                return Err(unknown_field(field, field_names, "field"));
            }
        }
        PolicyCheckExpr::And(parts) | PolicyCheckExpr::Or(parts) => {
            for part in parts {
                validate_policy_check(part, field_names)?;
            }
        }
        PolicyCheckExpr::Always
        | PolicyCheckExpr::ActorPresent
        | PolicyCheckExpr::ActorAttributeEquals { .. } => {}
    }
    Ok(())
}

fn validate_policy_effect(effect: &PolicyEffectSpec, field_names: &[String]) -> Result<()> {
    let check = match effect {
        PolicyEffectSpec::AuthorizeIf(c)
        | PolicyEffectSpec::AuthorizeUnless(c)
        | PolicyEffectSpec::ForbidIf(c)
        | PolicyEffectSpec::ForbidUnless(c) => c,
    };
    validate_policy_check(check, field_names)
}

fn validate_calc_fields(
    expr: &CalculationExprSpec,
    attr_names: &[String],
    arg_names: &[String],
) -> Result<()> {
    let candidates = {
        let mut names = attr_names.to_vec();
        names.extend(arg_names.iter().cloned());
        names
    };
    match expr {
        CalculationExprSpec::Field(ident) => {
            let name = ident.to_string();
            if !candidates.iter().any(|n| n == &name) {
                return Err(unknown_field(ident, &candidates, "field"));
            }
        }
        CalculationExprSpec::StringLength(field) => {
            if !candidates.iter().any(|n| n == &field.to_string()) {
                return Err(unknown_field(field, &candidates, "field"));
            }
        }
        CalculationExprSpec::Add(left, right)
        | CalculationExprSpec::Sub(left, right)
        | CalculationExprSpec::Mul(left, right)
        | CalculationExprSpec::Div(left, right)
        | CalculationExprSpec::Eq(left, right)
        | CalculationExprSpec::Ne(left, right)
        | CalculationExprSpec::Gt(left, right)
        | CalculationExprSpec::Gte(left, right)
        | CalculationExprSpec::Lt(left, right)
        | CalculationExprSpec::Lte(left, right) => {
            validate_calc_fields(left, attr_names, arg_names)?;
            validate_calc_fields(right, attr_names, arg_names)?;
        }
        CalculationExprSpec::Concat(parts) | CalculationExprSpec::Coalesce(parts) => {
            for part in parts {
                validate_calc_fields(part, attr_names, arg_names)?;
            }
        }
        CalculationExprSpec::Lower(inner)
        | CalculationExprSpec::Upper(inner)
        | CalculationExprSpec::Length(inner) => {
            validate_calc_fields(inner, attr_names, arg_names)?;
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_calc_fields(cond, attr_names, arg_names)?;
            validate_calc_fields(then_expr, attr_names, arg_names)?;
            validate_calc_fields(else_expr, attr_names, arg_names)?;
        }
        CalculationExprSpec::Arg(_)
        | CalculationExprSpec::LitInt(_)
        | CalculationExprSpec::LitString(_)
        | CalculationExprSpec::LitBool(_)
        | CalculationExprSpec::Null
        | CalculationExprSpec::Custom(_) => {}
    }
    Ok(())
}

fn check_multiple_primary_actions(def: &ResourceDefinition) -> Result<()> {
    let mut seen: Vec<(ActionKind, &Ident)> = Vec::new();
    for act in &def.actions {
        if !act.primary {
            continue;
        }
        if let Some((_, first)) = seen.iter().find(|(kind, _)| *kind == act.kind) {
            return Err(Error::new_spanned(
                &act.name,
                format!(
                    "multiple primary actions of kind '{}' on resource '{}' ('{}' and '{}') - only one primary action is allowed per kind",
                    act.kind.as_str(),
                    def.resource,
                    first,
                    act.name
                ),
            ));
        }
        seen.push((act.kind, &act.name));
    }
    Ok(())
}

fn validate_cross_section(def: &ResourceDefinition) -> Result<()> {
    check_multiple_primary_actions(def)?;

    let action_names: Vec<String> = def.actions.iter().map(|a| a.name.to_string()).collect();
    let attr_names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
    let rel_names: Vec<String> = def
        .relationships
        .iter()
        .map(|r| r.ident.to_string())
        .collect();
    let policy_fields = attr_and_rel_names(def);

    for pol in &def.policies {
        for when in &pol.whens {
            if let PolicyWhenSpec::ActionName(act_name) = when
                && !action_names.iter().any(|n| n == &act_name.to_string())
            {
                return Err(unknown_field(act_name, &action_names, "action"));
            }
        }
        for check in &pol.checks {
            validate_policy_effect(check, &policy_fields)?;
        }
    }

    for fp in &def.field_policies {
        if !attr_names.iter().any(|n| n == &fp.field.to_string()) {
            return Err(unknown_field(&fp.field, &attr_names, "attribute"));
        }
        for check in &fp.checks {
            validate_policy_effect(check, &policy_fields)?;
        }
    }

    for rel in &def.relationships {
        if rel.kind != RelType::BelongsTo {
            continue;
        }
        let fk_ident = rel
            .fk
            .clone()
            .unwrap_or_else(|| Ident::new(&format!("{}_id", rel.ident), rel.ident.span()));
        let fk = fk_ident.to_string();
        if attr_names.iter().any(|n| n == &fk) {
            continue;
        }
        let span = fk_ident.span();
        let suggestion = find_closest_match(&fk, ident_refs(&attr_names));
        let msg = if let Some(closest) = suggestion {
            format!(
                "belongs_to relationship `{}` requires foreign key `{fk}`, but no such attribute exists. Did you mean `{closest}`? Define `{fk}: Uuid`.",
                rel.ident
            )
        } else {
            format!(
                "belongs_to relationship `{}` requires foreign key `{fk}`, but no such attribute exists. Define `{fk}: Uuid`.",
                rel.ident
            )
        };
        return Err(Error::new(span, msg));
    }

    for calc in &def.calculations {
        let arg_names: Vec<String> = calc.arguments.iter().map(|a| a.name.to_string()).collect();
        validate_calc_fields(&calc.expr, &attr_names, &arg_names)?;
    }

    for agg in &def.aggregates {
        if !rel_names.iter().any(|n| n == &agg.relationship.to_string()) {
            return Err(unknown_field(&agg.relationship, &rel_names, "relationship"));
        }
    }

    Ok(())
}

fn type_label(ty: &Type) -> String {
    quote!(#ty).to_string().replace(' ', "")
}

fn is_string_type(ty: &Type) -> bool {
    is_string(ty) || option_inner(ty).is_some_and(is_string)
}

fn is_numeric_type(ty: &Type) -> bool {
    is_i64(ty)
        || is_integer(ty)
        || option_inner(ty).is_some_and(|inner| is_i64(inner) || is_integer(inner))
}

fn lint_uncovered_actions(def: &mut ResourceDefinition) {
    if def.policies.is_empty() {
        return;
    }
    for act in &def.actions {
        let covered = def.policies.iter().any(|pol| {
            pol.whens.iter().any(|when| match when {
                PolicyWhenSpec::Always => true,
                PolicyWhenSpec::ActionKind(kind) => *kind == act.kind,
                PolicyWhenSpec::ActionName(name) => name == &act.name,
            })
        });
        if !covered {
            def.warnings.push(crate::ast_helpers::make_deprecated_warning(
                act.name.span(),
                &format!(
                    "Action '{}' has no matching policy rule and will always be forbidden at runtime",
                    act.name
                ),
            ));
        }
    }
}

fn check_unique_idents<'a>(idents: impl IntoIterator<Item = &'a Ident>, kind: &str) -> Result<()> {
    let mut seen = HashSet::new();
    for ident in idents {
        let name = ident.to_string();
        if !seen.insert(name.clone()) {
            return Err(Error::new_spanned(
                ident,
                format!("duplicate {kind} `{name}`"),
            ));
        }
    }
    Ok(())
}

fn check_name_collisions(def: &ResourceDefinition) -> Result<()> {
    check_unique_idents(def.attributes.iter().map(|a| &a.ident), "attribute")?;
    check_unique_idents(def.actions.iter().map(|a| &a.name), "action")?;
    check_unique_idents(def.identities.iter().map(|i| &i.name), "identity")?;

    let attr_names: HashSet<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
    for rel in &def.relationships {
        let name = rel.ident.to_string();
        if attr_names.contains(&name) {
            return Err(Error::new_spanned(
                &rel.ident,
                format!("name collision: `{name}` is both an attribute and a relationship"),
            ));
        }
    }
    Ok(())
}

pub fn expand_define(mut def: ResourceDefinition) -> Result<TokenStream> {
    check_name_collisions(&def)?;

    // 0. Resolve accept field types from def.attributes for bracketed accept lists,
    // and validate all accept, change, validation, preparation, and identity fields.
    for action in &mut def.actions {
        if action.kind != crate::define::ast::ActionKind::Generic {
            for acc in &mut action.accept {
                let attr = def.attributes.iter().find(|a| a.ident == acc.name);
                if let Some(attr) = attr {
                    if acc.inferred {
                        acc.ty = attr.ty.clone();
                    }
                } else {
                    let attr_names: Vec<String> =
                        def.attributes.iter().map(|a| a.ident.to_string()).collect();
                    let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &acc.name,
                        &attr_refs,
                        "attribute",
                    ));
                }
            }
        }

        for chg in &action.changes {
            match chg {
                crate::define::ast::ChangeSpec::Set { field, .. }
                | crate::define::ast::ChangeSpec::SetNew { field, .. } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(
                            field,
                            &attr_refs,
                            "attribute",
                        ));
                    }
                }
                crate::define::ast::ChangeSpec::SetFromArg { field, argument } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(
                            field,
                            &attr_refs,
                            "attribute",
                        ));
                    }
                    if action.arguments.iter().all(|a| a.name != *argument) {
                        let arg_names: Vec<String> = action
                            .arguments
                            .iter()
                            .map(|a| a.name.to_string())
                            .collect();
                        let arg_refs: Vec<&str> = arg_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(
                            argument, &arg_refs, "argument",
                        ));
                    }
                }
                crate::define::ast::ChangeSpec::RelateActor { field } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                        return Err(crate::ast_helpers::unknown_ident_error(
                            field,
                            &attr_refs,
                            "attribute",
                        ));
                    }
                }
                crate::define::ast::ChangeSpec::ManageRelationship { relationship, .. }
                    if def.relationships.iter().all(|r| r.ident != *relationship) =>
                {
                    let rel_names: Vec<String> = def
                        .relationships
                        .iter()
                        .map(|r| r.ident.to_string())
                        .collect();
                    let rel_refs: Vec<&str> = rel_names.iter().map(|s| s.as_str()).collect();
                    return Err(crate::ast_helpers::unknown_ident_error(
                        relationship,
                        &rel_refs,
                        "relationship",
                    ));
                }
                _ => {}
            }
        }

        for prep in &action.preparations {
            if let crate::define::ast::PreparationSpec::Sort { field, .. } = prep
                && def.attributes.iter().all(|a| a.ident != *field)
            {
                let attr_names: Vec<String> =
                    def.attributes.iter().map(|a| a.ident.to_string()).collect();
                let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                return Err(crate::ast_helpers::unknown_ident_error(
                    field,
                    &attr_refs,
                    "attribute",
                ));
            }
        }

        for val in &action.validations {
            let field = match val {
                ValidationSpec::Present { field }
                | ValidationSpec::StringLength { field, .. }
                | ValidationSpec::OneOf { field, .. }
                | ValidationSpec::Numericality { field, .. } => Some(field),
                _ => None,
            };
            if let Some(field) = field {
                let attr = def.attributes.iter().find(|a| a.ident == *field);
                let is_arg = action.arguments.iter().any(|a| a.name == *field);
                if attr.is_none() && !is_arg {
                    let mut candidates: Vec<String> =
                        def.attributes.iter().map(|a| a.ident.to_string()).collect();
                    candidates.extend(action.arguments.iter().map(|a| a.name.to_string()));
                    let cand_refs: Vec<&str> = candidates.iter().map(|s| s.as_str()).collect();
                    return Err(crate::ast_helpers::unknown_ident_error(
                        field,
                        &cand_refs,
                        "attribute or argument",
                    ));
                }
                if let Some(attr) = attr {
                    match val {
                        ValidationSpec::StringLength { .. } if !is_string_type(&attr.ty) => {
                            return Err(Error::new_spanned(
                                field,
                                format!(
                                    "string_length validation cannot be applied to attribute '{field}' of type '{}'",
                                    type_label(&attr.ty)
                                ),
                            ));
                        }
                        ValidationSpec::Numericality { min, max, .. } => {
                            if min.is_none() && max.is_none() {
                                return Err(Error::new_spanned(
                                    field,
                                    format!(
                                        "numericality validation for '{field}' must specify at least one of 'min' or 'max'"
                                    ),
                                ));
                            }
                            if !is_numeric_type(&attr.ty) {
                                return Err(Error::new_spanned(
                                    field,
                                    format!(
                                        "numericality validation cannot be applied to attribute '{field}' of non-numeric type '{}'",
                                        type_label(&attr.ty)
                                    ),
                                ));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    for identity in &def.identities {
        for key in &identity.keys {
            if def.attributes.iter().all(|a| a.ident != *key) {
                let attr_names: Vec<String> =
                    def.attributes.iter().map(|a| a.ident.to_string()).collect();
                let attr_refs: Vec<&str> = attr_names.iter().map(|s| s.as_str()).collect();
                return Err(crate::ast_helpers::unknown_ident_error(
                    key,
                    &attr_refs,
                    "attribute",
                ));
            }
        }
    }

    validate_cross_section(&def)?;
    lint_uncovered_actions(&mut def);

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
        assert!(
            err.to_string().contains("Did you mean `subject`?"),
            "got: {}",
            err
        );
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
        assert!(
            err.to_string().contains("Did you mean `subject`?"),
            "got: {}",
            err
        );
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
        assert!(
            err.to_string().contains("Did you mean `reason_input`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_accept_brace_form_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                create open {
                    accept {
                        subjet: String,
                    }
                }
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `subject`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_set_change_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                status: String,
            }
            actions {
                create open {
                    change set(statu = "open");
                }
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `status`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_sort_preparation_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            actions {
                read read {
                    primary;
                    prepare sort(subjet);
                }
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `subject`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_identity_key_unknown_attribute_suggests_did_you_mean() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                email: String,
            }
            identities {
                identity unique_email: [emai];
            }
        };
        let def = parse_def(tokens);
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `email`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_duplicate_attribute_names_fail() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
                title: String,
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("duplicate attribute `title`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_duplicate_action_names_fail() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open { primary; }
                update open { primary; }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("duplicate action `open`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_duplicate_identity_names_fail() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                email: String,
            }
            identities {
                identity unique_email: [email];
                identity unique_email: [id];
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string()
                .contains("duplicate identity `unique_email`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_attribute_relationship_name_collision_fails() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                user: Uuid,
            }
            relationships {
                belongs_to user: User [fk: "user"];
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string()
                .contains("name collision: `user` is both an attribute and a relationship"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_attribute_error_lists_available_candidates() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                subject: String,
                status: String,
                opener_id: Uuid,
            }
            actions {
                create open {
                    accept [subjet];
                }
            }
        });
        let err = expand_err(def);
        let msg = err.to_string();
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
        assert!(
            msg.contains("Available attributes: id, subject, status, opener_id"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_policy_unknown_action_name_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open { primary; }
                read read { primary; }
            }
            policies {
                policy action(opn) {
                    authorize_if always;
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `open`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_policy_unknown_check_field_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                author_id: Uuid,
            }
            relationships {
                belongs_to author: User [fk: author_id];
            }
            policies {
                policy always {
                    authorize_if relates_to(auther);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `author`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_policy_is_nil_unknown_field_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                assignee_id: Option<Uuid>,
            }
            policies {
                policy always {
                    authorize_if is_nil(assignee);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `assignee_id`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_belongs_to_missing_fk_suggests_attribute_or_define() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                authorid: Uuid,
            }
            relationships {
                belongs_to author: User;
            }
        });
        let err = expand_err(def);
        let msg = err.to_string();
        assert!(msg.contains("author_id"), "got: {msg}");
        assert!(
            msg.contains("Did you mean `authorid`?") || msg.contains("Define `author_id: Uuid`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_belongs_to_explicit_fk_typo_suggests_attribute() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                author_id: Uuid,
            }
            relationships {
                belongs_to author: User [fk: auther_id];
            }
        });
        let err = expand_err(def);
        let msg = err.to_string();
        assert!(msg.contains("Did you mean `author_id`?"), "got: {msg}");
        assert!(msg.contains("Define `auther_id: Uuid`"), "got: {msg}");
    }

    #[test]
    fn test_belongs_to_matching_fk_expands() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                author_id: Uuid,
            }
            relationships {
                belongs_to author: User [fk: author_id];
            }
            actions {
                read read { primary; }
            }
        });
        expand_define(def).expect("valid belongs_to should expand");
    }

    #[test]
    fn test_calculation_unknown_field_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
            }
            calculations {
                title_len: i64 = string_length(titel);
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `title`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_calculation_unknown_field_ident_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
            }
            calculations {
                titled: String = titel;
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `title`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_aggregate_unknown_relationship_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            relationships {
                has_many comments: Vec<Comment>;
            }
            aggregates {
                comment_count: i64 = count(commnts);
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `comments`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_field_policy_unknown_target_suggests_did_you_mean() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
            }
            field_policies {
                field titel {
                    authorize_if always;
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains("Did you mean `title`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_multiple_primary_actions_of_same_kind_fail() {
        let def = parse_def(quote! {
            resource Ticket;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open { primary; }
                create draft { primary; }
            }
        });
        let err = expand_err(def);
        let msg = err.to_string();
        assert!(
            msg.contains("multiple primary actions of kind 'create' on resource 'Ticket' ('open' and 'draft')"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_string_length_on_integer_attribute_fails() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                count: i64,
            }
            actions {
                create open {
                    validate string_length(count, min = 1);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains(
                "string_length validation cannot be applied to attribute 'count' of type 'i64'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_numericality_on_string_attribute_fails() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
            }
            actions {
                create open {
                    validate numericality(title, min = 1);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains(
                "numericality validation cannot be applied to attribute 'title' of non-numeric type 'String'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_string_length_on_bool_or_uuid_attribute_fails() {
        let bool_def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                active: bool,
            }
            actions {
                create open {
                    validate string_length(active, min = 1);
                }
            }
        });
        let err = expand_err(bool_def);
        assert!(
            err.to_string().contains(
                "string_length validation cannot be applied to attribute 'active' of type 'bool'"
            ),
            "got: {}",
            err
        );

        let uuid_def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open {
                    validate string_length(id, min = 1);
                }
            }
        });
        let err = expand_err(uuid_def);
        assert!(
            err.to_string().contains(
                "string_length validation cannot be applied to attribute 'id' of type 'Uuid'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_numericality_on_bool_attribute_fails() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                active: bool,
            }
            actions {
                create open {
                    validate numericality(active, min = 0);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains(
                "numericality validation cannot be applied to attribute 'active' of non-numeric type 'bool'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_numericality_on_uuid_attribute_fails() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open {
                    validate numericality(id, min = 1);
                }
            }
        });
        let err = expand_err(def);
        assert!(
            err.to_string().contains(
                "numericality validation cannot be applied to attribute 'id' of non-numeric type 'Uuid'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_uncovered_action_emits_forbidden_warning() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
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
        });
        let out = expand_define(def).expect("expand").to_string();
        assert!(
            out.contains(
                "Action 'assign' has no matching policy rule and will always be forbidden at runtime"
            ),
            "missing uncovered action warning: {out}"
        );
        assert!(
            !out.contains("Action 'open' has no matching policy rule"),
            "open should be covered: {out}"
        );
        assert!(
            !out.contains("Action 'read' has no matching policy rule"),
            "read should be covered: {out}"
        );
    }

    #[test]
    fn test_policy_always_covers_all_actions() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
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
        });
        let out = expand_define(def).expect("expand").to_string();
        assert!(
            !out.contains("has no matching policy rule"),
            "always should cover all actions: {out}"
        );
    }

    #[test]
    fn test_attribute_docs_copied_to_field_consts_and_setters() {
        let def = parse_def(quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                /// The ticket subject
                subject: String,
            }
            actions {
                create open {
                    accept [subject];
                    /// Reason supplied by the caller
                    argument reason: String;
                }
            }
        });
        let out = expand_define(def).expect("expand").to_string();
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
