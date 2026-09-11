use crate::ast_helpers::{
    find_closest_match, is_bool, is_i64, is_integer, is_string, is_uuid, option_inner,
    unknown_ident_error,
};
use crate::define::ast::{
    ActionKind, CalculationExprSpec, PolicyCheckExpr, PolicyEffectSpec, PolicyWhenSpec, RelType,
    ResourceDefinition, ValidationSpec,
};
use quote::quote;
use std::collections::HashSet;
use syn::{Error, Ident, Type};

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

fn check_unique_idents<'a>(
    idents: impl IntoIterator<Item = &'a Ident>,
    kind: &str,
    errors: &mut Vec<Error>,
) {
    let mut seen = HashSet::new();
    for ident in idents {
        let name = ident.to_string();
        if !seen.insert(name.clone()) {
            errors.push(Error::new_spanned(
                ident,
                format!("duplicate {kind} `{name}`"),
            ));
        }
    }
}

fn check_name_collisions(def: &ResourceDefinition, errors: &mut Vec<Error>) {
    check_unique_idents(def.attributes.iter().map(|a| &a.ident), "attribute", errors);
    check_unique_idents(def.actions.iter().map(|a| &a.name), "action", errors);
    check_unique_idents(def.identities.iter().map(|i| &i.name), "identity", errors);

    let attr_names: HashSet<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
    for rel in &def.relationships {
        let name = rel.ident.to_string();
        if attr_names.contains(&name) {
            errors.push(Error::new_spanned(
                &rel.ident,
                format!("name collision: `{name}` is both an attribute and a relationship"),
            ));
        }
    }
}

fn validate_policy_check(
    check: &PolicyCheckExpr,
    field_names: &[String],
    actor: Option<&crate::define::ast::ActorSpec>,
    errors: &mut Vec<Error>,
) {
    match check {
        PolicyCheckExpr::RelatesToActor(field)
        | PolicyCheckExpr::IsNil(field)
        | PolicyCheckExpr::Eq { field, .. } => {
            if !field_names.iter().any(|n| n == &field.to_string()) {
                errors.push(unknown_field(field, field_names, "field"));
            }
        }
        PolicyCheckExpr::ActorAttributeEquals { attr, .. } => match actor {
            None => errors.push(Error::new_spanned(
                attr,
                format!(
                    "declare `actor {{ {attr}: Type; }}` to type-check actor attributes used in `actor_eq`"
                ),
            )),
            Some(spec) => {
                if spec.fields.iter().all(|f| f.name != *attr) {
                    let names: Vec<String> = spec.fields.iter().map(|f| f.name.to_string()).collect();
                    errors.push(unknown_ident_error(attr, &ident_refs(&names), "actor field"));
                }
            }
        },
        PolicyCheckExpr::And(parts) | PolicyCheckExpr::Or(parts) => {
            for part in parts {
                validate_policy_check(part, field_names, actor, errors);
            }
        }
        PolicyCheckExpr::Always | PolicyCheckExpr::ActorPresent => {}
    }
}

fn validate_policy_effect(
    effect: &PolicyEffectSpec,
    field_names: &[String],
    actor: Option<&crate::define::ast::ActorSpec>,
    errors: &mut Vec<Error>,
) {
    let check = match effect {
        PolicyEffectSpec::AuthorizeIf(c)
        | PolicyEffectSpec::AuthorizeUnless(c)
        | PolicyEffectSpec::ForbidIf(c)
        | PolicyEffectSpec::ForbidUnless(c) => c,
    };
    validate_policy_check(check, field_names, actor, errors);
}

fn validate_calc_fields(
    expr: &CalculationExprSpec,
    attr_names: &[String],
    arg_names: &[String],
    errors: &mut Vec<Error>,
) {
    let candidates = {
        let mut names = attr_names.to_vec();
        names.extend(arg_names.iter().cloned());
        names
    };
    match expr {
        CalculationExprSpec::Field(ident) => {
            let name = ident.to_string();
            if !candidates.iter().any(|n| n == &name) {
                errors.push(unknown_field(ident, &candidates, "field"));
            }
        }
        CalculationExprSpec::StringLength(field) => {
            if !candidates.iter().any(|n| n == &field.to_string()) {
                errors.push(unknown_field(field, &candidates, "field"));
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
            validate_calc_fields(left, attr_names, arg_names, errors);
            validate_calc_fields(right, attr_names, arg_names, errors);
        }
        CalculationExprSpec::Concat(parts) | CalculationExprSpec::Coalesce(parts) => {
            for part in parts {
                validate_calc_fields(part, attr_names, arg_names, errors);
            }
        }
        CalculationExprSpec::Lower(inner)
        | CalculationExprSpec::Upper(inner)
        | CalculationExprSpec::Length(inner) => {
            validate_calc_fields(inner, attr_names, arg_names, errors);
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_calc_fields(cond, attr_names, arg_names, errors);
            validate_calc_fields(then_expr, attr_names, arg_names, errors);
            validate_calc_fields(else_expr, attr_names, arg_names, errors);
        }
        CalculationExprSpec::Arg(_)
        | CalculationExprSpec::LitInt(_)
        | CalculationExprSpec::LitString(_)
        | CalculationExprSpec::LitBool(_)
        | CalculationExprSpec::Null
        | CalculationExprSpec::Custom(_) => {}
    }
}

fn check_multiple_primary_actions(def: &ResourceDefinition, errors: &mut Vec<Error>) {
    let mut seen: Vec<(ActionKind, &Ident)> = Vec::new();
    for act in &def.actions {
        if !act.primary {
            continue;
        }
        if let Some((_, first)) = seen.iter().find(|(kind, _)| *kind == act.kind) {
            errors.push(Error::new_spanned(
                &act.name,
                format!(
                    "multiple primary actions of kind '{}' on resource '{}' ('{}' and '{}') - only one primary action is allowed per kind",
                    act.kind.as_str(),
                    def.resource,
                    first,
                    act.name
                ),
            ));
        } else {
            seen.push((act.kind, &act.name));
        }
    }
}

fn validate_cross_section(def: &ResourceDefinition, errors: &mut Vec<Error>) {
    check_multiple_primary_actions(def, errors);

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
                errors.push(unknown_field(act_name, &action_names, "action"));
            }
        }
        for check in &pol.checks {
            validate_policy_effect(check, &policy_fields, def.actor.as_ref(), errors);
        }
    }

    for fp in &def.field_policies {
        if !attr_names.iter().any(|n| n == &fp.field.to_string()) {
            errors.push(unknown_field(&fp.field, &attr_names, "attribute"));
        }
        for check in &fp.checks {
            validate_policy_effect(check, &policy_fields, def.actor.as_ref(), errors);
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
        errors.push(Error::new(span, msg));
    }

    for calc in &def.calculations {
        let arg_names: Vec<String> = calc.arguments.iter().map(|a| a.name.to_string()).collect();
        validate_calc_fields(&calc.expr, &attr_names, &arg_names, errors);
    }

    for agg in &def.aggregates {
        if !rel_names.iter().any(|n| n == &agg.relationship.to_string()) {
            errors.push(unknown_field(&agg.relationship, &rel_names, "relationship"));
        }
    }
}

fn lint_uncovered_actions(def: &ResourceDefinition, errors: &mut Vec<Error>) {
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
            errors.push(Error::new_spanned(
                &act.name,
                format!(
                    "Action '{}' has no matching policy rule and will always be forbidden at runtime",
                    act.name
                ),
            ));
        }
    }
}

fn validate_kind_gates(action: &crate::define::ast::ActionSpec, errors: &mut Vec<Error>) {
    use crate::define::ast::ChangeSpec;
    let kind = action.kind;
    let kind_name = kind.as_str();

    if !action.preparations.is_empty() && kind != ActionKind::Read {
        errors.push(Error::new_spanned(
            &action.name,
            format!("`prepare` is only valid on `read` actions, not `{kind_name}`"),
        ));
    }
    if !action.accept.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            &action.name,
            "`accept` is not valid on `read` actions",
        ));
    }
    if !action.changes.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            &action.name,
            "`change` is not valid on `read` actions",
        ));
    }
    if !action.validations.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            &action.name,
            "`validate` is not valid on `read` actions",
        ));
    }
    if action.persist_manual && kind != ActionKind::Create {
        errors.push(Error::new_spanned(
            &action.name,
            "`persist manual` is only valid on `create` actions",
        ));
    }
    if action.returns.is_some() && kind != ActionKind::Generic {
        errors.push(Error::new_spanned(
            &action.name,
            "`returns` is only valid on `generic` actions",
        ));
    }
    if action.run_expr.is_some() && kind != ActionKind::Generic {
        errors.push(Error::new_spanned(
            &action.name,
            "`run` is only valid on `generic` actions",
        ));
    }
    if kind == ActionKind::Generic {
        for chg in &action.changes {
            if matches!(chg, ChangeSpec::RelateActor { .. }) {
                errors.push(Error::new_spanned(
                    &action.name,
                    "`relate_actor` is not valid on `generic` actions",
                ));
            }
        }
    }
}

fn validate_actions(def: &mut ResourceDefinition, errors: &mut Vec<Error>) {
    for action in &mut def.actions {
        validate_kind_gates(action, errors);
        if action.kind != ActionKind::Generic {
            let mut kept = Vec::new();
            for acc in std::mem::take(&mut action.accept) {
                if let Some(attr) = def.attributes.iter().find(|a| a.ident == acc.name) {
                    let mut acc = acc;
                    if acc.inferred {
                        acc.ty = attr.ty.clone();
                    }
                    kept.push(acc);
                } else {
                    let attr_names: Vec<String> =
                        def.attributes.iter().map(|a| a.ident.to_string()).collect();
                    errors.push(unknown_ident_error(
                        &acc.name,
                        &ident_refs(&attr_names),
                        "attribute",
                    ));
                }
            }
            action.accept = kept;
        }

        let mut kept_changes = Vec::new();
        for chg in std::mem::take(&mut action.changes) {
            use crate::define::ast::ChangeSpec;
            match &chg {
                ChangeSpec::Set { field, .. } | ChangeSpec::SetNew { field, .. } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        errors.push(unknown_ident_error(
                            field,
                            &ident_refs(&attr_names),
                            "attribute",
                        ));
                    } else {
                        kept_changes.push(chg);
                    }
                }
                ChangeSpec::SetFromArg { field, argument } => {
                    let mut keep = true;
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        errors.push(unknown_ident_error(
                            field,
                            &ident_refs(&attr_names),
                            "attribute",
                        ));
                        keep = false;
                    }
                    if action.arguments.iter().all(|a| a.name != *argument) {
                        let arg_names: Vec<String> = action
                            .arguments
                            .iter()
                            .map(|a| a.name.to_string())
                            .collect();
                        errors.push(unknown_ident_error(
                            argument,
                            &ident_refs(&arg_names),
                            "argument",
                        ));
                        keep = false;
                    }
                    if keep {
                        kept_changes.push(chg);
                    }
                }
                ChangeSpec::RelateActor { field } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        errors.push(unknown_ident_error(
                            field,
                            &ident_refs(&attr_names),
                            "attribute",
                        ));
                    } else {
                        kept_changes.push(chg);
                    }
                }
                ChangeSpec::ManageRelationship { relationship, .. } => {
                    if def.relationships.iter().all(|r| r.ident != *relationship) {
                        let rel_names: Vec<String> = def
                            .relationships
                            .iter()
                            .map(|r| r.ident.to_string())
                            .collect();
                        errors.push(unknown_ident_error(
                            relationship,
                            &ident_refs(&rel_names),
                            "relationship",
                        ));
                    } else {
                        kept_changes.push(chg);
                    }
                }
                _ => kept_changes.push(chg),
            }
        }
        action.changes = kept_changes;

        for prep in &action.preparations {
            if let crate::define::ast::PreparationSpec::Sort { field, .. } = prep
                && def.attributes.iter().all(|a| a.ident != *field)
            {
                let attr_names: Vec<String> =
                    def.attributes.iter().map(|a| a.ident.to_string()).collect();
                errors.push(unknown_ident_error(
                    field,
                    &ident_refs(&attr_names),
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
                    errors.push(unknown_ident_error(
                        field,
                        &ident_refs(&candidates),
                        "attribute or argument",
                    ));
                }
                if let Some(attr) = attr {
                    match val {
                        ValidationSpec::OneOf { .. }
                            if attr.uses_ash_type_storage() || attr.is_enum =>
                        {
                            errors.push(Error::new_spanned(
                                field,
                                format!(
                                    "`one_of` is not valid on enum attribute `{field}` — variants already constrain the value"
                                ),
                            ));
                        }
                        ValidationSpec::StringLength { .. } if !is_string_type(&attr.ty) => {
                            errors.push(Error::new_spanned(
                                field,
                                format!(
                                    "string_length validation cannot be applied to attribute '{field}' of type '{}'",
                                    type_label(&attr.ty)
                                ),
                            ));
                        }
                        ValidationSpec::Numericality { min, max, .. } => {
                            if min.is_none() && max.is_none() {
                                errors.push(Error::new_spanned(
                                    field,
                                    format!(
                                        "numericality validation for '{field}' must specify at least one of 'min' or 'max'"
                                    ),
                                ));
                            }
                            if !is_numeric_type(&attr.ty) {
                                errors.push(Error::new_spanned(
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
                errors.push(unknown_ident_error(
                    key,
                    &ident_refs(&attr_names),
                    "attribute",
                ));
            }
        }
    }
}

/// Resolve inferred types, collect every semantic error, and drop invalid field refs so
/// codegen can still emit a usable struct.
fn validate_attributes(def: &ResourceDefinition, errors: &mut Vec<Error>) {
    for attr in &def.attributes {
        if !attr.is_enum {
            continue;
        }
        let inner = option_inner(&attr.ty).unwrap_or(&attr.ty);
        if is_uuid(inner) || is_string(inner) || is_i64(inner) || is_bool(inner) {
            errors.push(Error::new_spanned(
                &attr.ty,
                "`[enum]` requires a type that implements `AshEnum`, not a builtin scalar",
            ));
        }
    }
}

pub fn validate(def: &mut ResourceDefinition) -> Vec<Error> {
    let mut errors = Vec::new();
    check_name_collisions(def, &mut errors);
    validate_attributes(def, &mut errors);
    validate_actions(def, &mut errors);
    validate_cross_section(def, &mut errors);
    lint_uncovered_actions(def, &mut errors);
    errors
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

    fn validate_err_msg(tokens: proc_macro2::TokenStream) -> String {
        let mut def = parse_def(tokens);
        let errors = validate(&mut def);
        if errors.is_empty() {
            panic!("expected validation errors");
        }
        errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_accept_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            actions {
                create open {
                    accept [subjet];
                }
            }
        }});
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
    }

    #[test]
    fn test_collects_multiple_errors() {
        let mut def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            actions {
                create open {
                    accept [subjet];
                    change set(statu = "open");
                }
            }
        }});
        let errors = validate(&mut def);
        assert!(
            errors.len() >= 2,
            "expected multiple errors, got {errors:?}"
        );
    }

    #[test]
    fn test_validation_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            actions {
                create open {
                    accept [subject];
                    validate present(subjet);
                }
            }
        }});
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
    }

    #[test]
    fn test_set_from_arg_unknown_argument_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                reason: String;
            }
            actions {
                create open {
                    argument reason_input: String;
                    change set_from_arg(reason, reason_inpt);
                }
            }
        }});
        assert!(msg.contains("Did you mean `reason_input`?"), "got: {msg}");
    }

    #[test]
    fn test_set_change_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                status: String;
            }
            actions {
                create open {
                    change set(statu = "open");
                }
            }
        }});
        assert!(msg.contains("Did you mean `status`?"), "got: {msg}");
    }

    #[test]
    fn test_sort_preparation_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            actions {
                read read {
                    primary;
                    prepare sort(subjet);
                }
            }
        }});
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
    }

    #[test]
    fn test_identity_key_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                email: String;
            }
            identities {
                identity unique_email: [emai];
            }
        }});
        assert!(msg.contains("Did you mean `email`?"), "got: {msg}");
    }

    #[test]
    fn test_duplicate_attribute_names_fail() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
                title: String;
            }
        }});
        assert!(msg.contains("duplicate attribute `title`"), "got: {msg}");
    }

    #[test]
    fn test_duplicate_action_names_fail() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            actions {
                create open { primary; }
                update open { primary; }
            }
        }});
        assert!(msg.contains("duplicate action `open`"), "got: {msg}");
    }

    #[test]
    fn test_duplicate_identity_names_fail() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                email: String;
            }
            identities {
                identity unique_email: [email];
                identity unique_email: [id];
            }
        }});
        assert!(
            msg.contains("duplicate identity `unique_email`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_attribute_relationship_name_collision_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                user: Uuid;
            }
            relationships {
                belongs_to user: User [fk: user];
            }
        }});
        assert!(
            msg.contains("name collision: `user` is both an attribute and a relationship"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_unknown_attribute_error_lists_available_candidates() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
                status: String;
                opener_id: Uuid;
            }
            actions {
                create open {
                    accept [subjet];
                }
            }
        }});
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
        assert!(
            msg.contains("Available attributes: id, subject, status, opener_id"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_policy_unknown_action_name_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
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
        }});
        assert!(msg.contains("Did you mean `open`?"), "got: {msg}");
    }

    #[test]
    fn test_policy_unknown_check_field_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                author_id: Uuid;
            }
            relationships {
                belongs_to author: User [fk: author_id];
            }
            policies {
                policy always {
                    authorize_if relates_to(auther);
                }
            }
        }});
        assert!(msg.contains("Did you mean `author`?"), "got: {msg}");
    }

    #[test]
    fn test_policy_is_nil_unknown_field_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                assignee_id: Option<Uuid>;
            }
            policies {
                policy always {
                    authorize_if is_nil(assignee);
                }
            }
        }});
        assert!(msg.contains("Did you mean `assignee_id`?"), "got: {msg}");
    }

    #[test]
    fn test_belongs_to_missing_fk_suggests_attribute_or_define() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                authorid: Uuid;
            }
            relationships {
                belongs_to author: User;
            }
        }});
        assert!(msg.contains("author_id"), "got: {msg}");
        assert!(
            msg.contains("Did you mean `authorid`?") || msg.contains("Define `author_id: Uuid`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_belongs_to_explicit_fk_typo_suggests_attribute() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                author_id: Uuid;
            }
            relationships {
                belongs_to author: User [fk: auther_id];
            }
        }});
        assert!(msg.contains("Did you mean `author_id`?"), "got: {msg}");
        assert!(msg.contains("Define `auther_id: Uuid`"), "got: {msg}");
    }

    #[test]
    fn test_calculation_unknown_field_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
            }
            calculations {
                title_len: i64 = string_length(titel);
            }
        }});
        assert!(msg.contains("Did you mean `title`?"), "got: {msg}");
    }

    #[test]
    fn test_calculation_unknown_field_ident_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
            }
            calculations {
                titled: String = titel;
            }
        }});
        assert!(msg.contains("Did you mean `title`?"), "got: {msg}");
    }

    #[test]
    fn test_aggregate_unknown_relationship_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            relationships {
                has_many comments: Comment;
            }
            aggregates {
                comment_count: i64 = count(commnts);
            }
        }});
        assert!(msg.contains("Did you mean `comments`?"), "got: {msg}");
    }

    #[test]
    fn test_field_policy_unknown_target_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
            }
            field_policies {
                field titel {
                    authorize_if always;
                }
            }
        }});
        assert!(msg.contains("Did you mean `title`?"), "got: {msg}");
    }

    #[test]
    fn test_multiple_primary_actions_of_same_kind_fail() {
        let msg = validate_err_msg(quote! {
            Ticket {
            attributes {
                id: Uuid [pk];
            }
            actions {
                create open { primary; }
                create draft { primary; }
            }
        }});
        assert!(
            msg.contains("multiple primary actions of kind 'create' on resource 'Ticket' ('open' and 'draft')"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_string_length_on_integer_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                count: i64;
            }
            actions {
                create open {
                    validate string_length(count, min: 1);
                }
            }
        }});
        assert!(
            msg.contains(
                "string_length validation cannot be applied to attribute 'count' of type 'i64'"
            ),
            "got: {msg}"
        );
    }

    #[test]
    fn test_numericality_on_string_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
            }
            actions {
                create open {
                    validate numericality(title, min: 1);
                }
            }
        }});
        assert!(
            msg.contains(
                "numericality validation cannot be applied to attribute 'title' of non-numeric type 'String'"
            ),
            "got: {msg}"
        );
    }

    #[test]
    fn test_string_length_on_bool_or_uuid_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                active: bool;
            }
            actions {
                create open {
                    validate string_length(active, min: 1);
                }
            }
        }});
        assert!(
            msg.contains(
                "string_length validation cannot be applied to attribute 'active' of type 'bool'"
            ),
            "got: {msg}"
        );

        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            actions {
                create open {
                    validate string_length(id, min: 1);
                }
            }
        }});
        assert!(
            msg.contains(
                "string_length validation cannot be applied to attribute 'id' of type 'Uuid'"
            ),
            "got: {msg}"
        );
    }

    #[test]
    fn test_numericality_on_bool_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                active: bool;
            }
            actions {
                create open {
                    validate numericality(active, min: 0);
                }
            }
        }});
        assert!(
            msg.contains(
                "numericality validation cannot be applied to attribute 'active' of non-numeric type 'bool'"
            ),
            "got: {msg}"
        );
    }

    #[test]
    fn test_numericality_on_uuid_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            actions {
                create open {
                    validate numericality(id, min: 1);
                }
            }
        }});
        assert!(
            msg.contains(
                "numericality validation cannot be applied to attribute 'id' of non-numeric type 'Uuid'"
            ),
            "got: {msg}"
        );
    }

    #[test]
    fn test_uncovered_action_is_an_error() {
        let msg = validate_err_msg(quote! {
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
        assert!(
            msg.contains(
                "Action 'assign' has no matching policy rule and will always be forbidden at runtime"
            ),
            "got: {msg}"
        );
        assert!(
            !msg.contains("Action 'open' has no matching policy rule"),
            "open should be covered: {msg}"
        );
    }

    #[test]
    fn test_actor_eq_requires_actor_block() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                }
                actions {
                    read read { primary; }
                }
                policies {
                    policy action_type(read) {
                        authorize_if actor_eq(role = "admin");
                    }
                }
            }
        });
        assert!(
            msg.contains("declare `actor { role: Type; }`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_one_of_on_enum_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    status: Status [enum];
                }
                actions {
                    create open {
                        accept [status];
                        validate one_of(status, ["open", "closed"]);
                    }
                }
            }
        });
        assert!(
            msg.contains("`one_of` is not valid on enum attribute `status`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_one_of_on_inferred_enum_attribute_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    status: Status;
                }
                actions {
                    create open {
                        accept [status];
                        validate one_of(status, ["open", "closed"]);
                    }
                }
            }
        });
        assert!(
            msg.contains("`one_of` is not valid on enum attribute `status`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_enum_flag_on_builtin_scalar_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    status: String [enum];
                }
                actions {
                    read read { primary; }
                }
            }
        });
        assert!(
            msg.contains("`[enum]` requires a type that implements `AshEnum`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_prepare_is_only_valid_on_read() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    archived: bool;
                }
                actions {
                    create open {
                        prepare filter(archived == false);
                    }
                }
            }
        });
        assert!(
            msg.contains("`prepare` is only valid on `read` actions, not `create`"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_accept_change_validate_forbidden_on_read() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    read read {
                        primary;
                        accept [title];
                        change set(title = "x");
                        validate present(title);
                    }
                }
            }
        });
        assert!(
            msg.contains("`accept` is not valid on `read` actions"),
            "got: {msg}"
        );
        assert!(
            msg.contains("`change` is not valid on `read` actions"),
            "got: {msg}"
        );
        assert!(
            msg.contains("`validate` is not valid on `read` actions"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_persist_manual_only_on_create() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    update assign {
                        persist manual;
                    }
                }
            }
        });
        assert!(
            msg.contains("`persist manual` is only valid on `create` actions"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_returns_and_run_only_on_generic() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    create open {
                        accept [title];
                        returns String;
                        run |_input| async move { Ok("x".into()) };
                    }
                }
            }
        });
        assert!(
            msg.contains("`returns` is only valid on `generic` actions"),
            "got: {msg}"
        );
        assert!(
            msg.contains("`run` is only valid on `generic` actions"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_relate_actor_forbidden_on_generic() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    opener_id: Uuid;
                }
                actions {
                    generic ping, bool {
                        change relate_actor(opener_id);
                        run |_input| async move { Ok(true) };
                    }
                }
            }
        });
        assert!(
            msg.contains("`relate_actor` is not valid on `generic` actions"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_generic_may_omit_run() {
        let mut def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                }
                actions {
                    generic dynamic_operation, String {
                        argument payload: String;
                    }
                }
            }
        });
        let errors = validate(&mut def);
        assert!(errors.is_empty(), "unexpected: {errors:?}");
    }
}
