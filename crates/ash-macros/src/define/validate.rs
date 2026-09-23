use crate::ast_helpers::{
    find_closest_match, is_bool, is_integer, is_string, is_uuid, option_inner, unknown_ident_error,
};
use crate::define::ast::{
    ActionKind, ArgumentSpec, CalculationExprSpec, PolicyCheckExpr, PolicyEffectSpec,
    PolicyWhenSpec, RelType, ResourceDefinition, ValidationSpec,
};
use quote::{quote, quote_spanned};
use std::collections::HashSet;
use syn::{Error, Ident, Type};

fn ident_refs(names: &[String]) -> Vec<&str> {
    names.iter().map(String::as_str).collect()
}

fn unknown_field(ident: &Ident, names: &[String], kind: &str) -> Error {
    unknown_ident_error(ident, &ident_refs(names), kind)
}

fn slot_hint(expected: &str, actual: &str) -> &'static str {
    match (expected, actual) {
        ("attribute", "relationship") => {
            " `accept` and `change set` take persisted attributes; use `change manage(...)` for related records."
        }
        ("attribute", "calculation") | ("attribute", "aggregate") => {
            " calculations and aggregates are computed, not written."
        }
        ("relationship", "attribute") => {
            " use the relationship name (`has_many comments`), not a foreign-key attribute."
        }
        ("action", _) => " `policy action(...)` takes an action name from this resource.",
        _ => "",
    }
}

struct SlotNames {
    attrs: Vec<String>,
    rels: Vec<String>,
    calcs: Vec<String>,
    aggs: Vec<String>,
    actions: Vec<String>,
}

fn slot_names(def: &ResourceDefinition) -> SlotNames {
    SlotNames {
        attrs: def.attributes.iter().map(|a| a.ident.to_string()).collect(),
        rels: def.relationships.iter().map(|r| r.ident.to_string()).collect(),
        calcs: def.calculations.iter().map(|c| c.ident.to_string()).collect(),
        aggs: def.aggregates.iter().map(|a| a.ident.to_string()).collect(),
        actions: def.actions.iter().map(|a| a.name.to_string()).collect(),
    }
}

fn slot_error(
    ident: &Ident,
    expected: &str,
    candidates: &[String],
    names: &SlotNames,
) -> Error {
    let SlotNames {
        attrs,
        rels,
        calcs,
        aggs,
        actions,
    } = names;
    let name = ident.to_string();
    let actual = if attrs.iter().any(|n| n == &name) {
        Some("attribute")
    } else if rels.iter().any(|n| n == &name) {
        Some("relationship")
    } else if calcs.iter().any(|n| n == &name) {
        Some("calculation")
    } else if aggs.iter().any(|n| n == &name) {
        Some("aggregate")
    } else if actions.iter().any(|n| n == &name) {
        Some("action")
    } else {
        None
    };
    if let Some(actual) = actual
        && actual != expected
    {
        return Error::new_spanned(
            ident,
            format!(
                "`{name}` is a {actual}, not an {expected}.{}",
                slot_hint(expected, actual)
            ),
        );
    }
    unknown_field(ident, candidates, expected)
}

fn is_filter_field_ident(id: &Ident) -> bool {
    let name = id.to_string();
    !matches!(name.as_str(), "true" | "false" | "Self" | "Some" | "None")
        && name
            .chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_lowercase())
}

fn collect_filter_idents<'a>(expr: &'a syn::Expr, out: &mut Vec<&'a Ident>) {
    match expr {
        syn::Expr::Binary(b) => {
            collect_filter_idents(&b.left, out);
            collect_filter_idents(&b.right, out);
        }
        syn::Expr::MethodCall(m) => {
            collect_filter_idents(&m.receiver, out);
            for arg in &m.args {
                collect_filter_idents(arg, out);
            }
        }
        syn::Expr::Unary(u) => collect_filter_idents(&u.expr, out),
        syn::Expr::Paren(p) => collect_filter_idents(&p.expr, out),
        syn::Expr::Group(g) => collect_filter_idents(&g.expr, out),
        syn::Expr::Path(p) => {
            if let Some(id) = p.path.get_ident() {
                if is_filter_field_ident(id) {
                    out.push(id);
                }
            } else if p.path.segments.len() == 2 && p.path.segments[0].ident == "Self" {
                let id = &p.path.segments[1].ident;
                if is_filter_field_ident(id) {
                    out.push(id);
                }
            }
        }
        _ => {}
    }
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
    is_integer(ty) || option_inner(ty).is_some_and(is_integer)
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

fn calc_operand_ty<'a>(
    name: &Ident,
    def: &'a ResourceDefinition,
    args: &'a [ArgumentSpec],
) -> Option<&'a Type> {
    args.iter()
        .find(|a| a.name == *name)
        .map(|a| &a.ty)
        .or_else(|| {
            def.attributes
                .iter()
                .find(|a| a.ident == *name)
                .map(|a| &a.ty)
        })
}

fn require_stringy_calc_field(
    field: &Ident,
    op: &str,
    def: &ResourceDefinition,
    args: &[ArgumentSpec],
    errors: &mut Vec<Error>,
) {
    if let Some(ty) = calc_operand_ty(field, def, args)
        && !is_string_type(ty)
    {
        errors.push(Error::new_spanned(
            field,
            format!(
                "`{op}({field})` requires a string field, but `{field}` has type '{}'",
                type_label(ty)
            ),
        ));
    }
}

fn require_stringy_calc_expr(
    expr: &CalculationExprSpec,
    op: &str,
    def: &ResourceDefinition,
    args: &[ArgumentSpec],
    errors: &mut Vec<Error>,
) {
    match expr {
        CalculationExprSpec::Field(field) | CalculationExprSpec::Arg(field) => {
            require_stringy_calc_field(field, op, def, args, errors);
        }
        CalculationExprSpec::LitString(_) => {}
        _ => {}
    }
}

fn validate_calc_fields(
    expr: &CalculationExprSpec,
    def: &ResourceDefinition,
    args: &[ArgumentSpec],
    errors: &mut Vec<Error>,
) {
    let candidates = {
        let mut names: Vec<String> = def.attributes.iter().map(|a| a.ident.to_string()).collect();
        names.extend(args.iter().map(|a| a.name.to_string()));
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
            } else {
                require_stringy_calc_field(field, "string_length", def, args, errors);
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
            validate_calc_fields(left, def, args, errors);
            validate_calc_fields(right, def, args, errors);
        }
        CalculationExprSpec::Concat(parts) | CalculationExprSpec::Coalesce(parts) => {
            for part in parts {
                validate_calc_fields(part, def, args, errors);
            }
        }
        CalculationExprSpec::Lower(inner) => {
            validate_calc_fields(inner, def, args, errors);
            require_stringy_calc_expr(inner, "lower", def, args, errors);
        }
        CalculationExprSpec::Upper(inner) => {
            validate_calc_fields(inner, def, args, errors);
            require_stringy_calc_expr(inner, "upper", def, args, errors);
        }
        CalculationExprSpec::Length(inner) => {
            validate_calc_fields(inner, def, args, errors);
            require_stringy_calc_expr(inner, "length", def, args, errors);
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            validate_calc_fields(cond, def, args, errors);
            validate_calc_fields(then_expr, def, args, errors);
            validate_calc_fields(else_expr, def, args, errors);
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
    let names = slot_names(def);

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
                errors.push(slot_error(
                    act_name,
                    "action",
                    &action_names,
                    &names,
                ));
            }
        }
        for check in &pol.checks {
            validate_policy_effect(check, &policy_fields, def.actor.as_ref(), errors);
        }
    }

    for fp in &def.field_policies {
        if !attr_names.iter().any(|n| n == &fp.field.to_string()) {
            errors.push(slot_error(
                &fp.field,
                "attribute",
                &attr_names,
                &names,
            ));
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
        validate_calc_fields(&calc.expr, def, &calc.arguments, errors);
    }

    for agg in &def.aggregates {
        if !rel_names.iter().any(|n| n == &agg.relationship.to_string()) {
            errors.push(slot_error(
                &agg.relationship,
                "relationship",
                &rel_names,
                &names,
            ));
        }
    }

    if let Some(lock) = &def.optimistic_lock
        && def.attributes.iter().all(|a| a.ident != *lock) {
            errors.push(slot_error(
                lock,
                "attribute",
                &attr_names,
                &names,
            ));
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
    let name = &action.name;
    let prepare_at = action.prepare_kw.as_ref().unwrap_or(name);
    let accept_at = action.accept_kw.as_ref().unwrap_or(name);
    let change_at = action.change_kw.as_ref().unwrap_or(name);
    let validate_at = action.validate_kw.as_ref().unwrap_or(name);
    let persist_at = action.persist_kw.as_ref().unwrap_or(name);
    let returns_at = action.returns_kw.as_ref().unwrap_or(name);
    let run_at = action.run_kw.as_ref().unwrap_or(name);

    if !action.preparations.is_empty() && kind != ActionKind::Read {
        errors.push(Error::new_spanned(
            prepare_at,
            format!("`prepare` is only valid on `read` actions, not `{kind_name}`"),
        ));
    }
    if !action.accept.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            accept_at,
            "`accept` is not valid on `read` actions",
        ));
    }
    if !action.changes.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            change_at,
            "`change` is not valid on `read` actions",
        ));
    }
    if !action.validations.is_empty() && kind == ActionKind::Read {
        errors.push(Error::new_spanned(
            validate_at,
            "`validate` is not valid on `read` actions",
        ));
    }
    if action.persist_manual && kind != ActionKind::Create {
        errors.push(Error::new_spanned(
            persist_at,
            "`persist manual` is only valid on `create` actions",
        ));
    }
    if action.returns.is_some() && kind != ActionKind::Generic {
        errors.push(Error::new_spanned(
            returns_at,
            "`returns` is only valid on `generic` actions",
        ));
    }
    if action.run_expr.is_some() && kind != ActionKind::Generic {
        errors.push(Error::new_spanned(
            run_at,
            "`run` is only valid on `generic` actions",
        ));
    }
    if kind == ActionKind::Generic {
        for chg in &action.changes {
            if let ChangeSpec::RelateActor { field } = chg {
                errors.push(Error::new_spanned(
                    field,
                    "`relate_actor` is not valid on `generic` actions",
                ));
            }
        }
    }
}

fn validate_actions(def: &mut ResourceDefinition, errors: &mut Vec<Error>) {
    let names = slot_names(def);
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
                    errors.push(slot_error(
                        &acc.name,
                        "attribute",
                        &attr_names,
                        &names,
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
                        errors.push(slot_error(
                            field,
                            "attribute",
                            &attr_names,
                            &names,
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
                        errors.push(slot_error(
                            field,
                            "attribute",
                            &attr_names,
                            &names,
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
                        errors.push(slot_error(
                            field,
                            "attribute",
                            &attr_names,
                            &names,
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
                        errors.push(slot_error(
                            relationship,
                            "relationship",
                            &rel_names,
                            &names,
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
            match prep {
                crate::define::ast::PreparationSpec::Sort { field, .. } => {
                    if def.attributes.iter().all(|a| a.ident != *field) {
                        let attr_names: Vec<String> =
                            def.attributes.iter().map(|a| a.ident.to_string()).collect();
                        errors.push(slot_error(
                            field,
                            "attribute",
                            &attr_names,
                            &names,
                        ));
                    }
                }
                crate::define::ast::PreparationSpec::Filter { expr } => {
                    let mut fields = Vec::new();
                    collect_filter_idents(expr, &mut fields);
                    let mut filterable = names.attrs.clone();
                    filterable.extend(names.calcs.iter().cloned());
                    filterable.extend(names.aggs.iter().cloned());
                    for field in fields {
                        let name = field.to_string();
                        if filterable.iter().any(|n| n == &name) {
                            continue;
                        }
                        if names.rels.iter().any(|n| n == &name) {
                            errors.push(Error::new_spanned(
                                field,
                                format!(
                                    "`{name}` is a relationship, not a filter field. `prepare filter` runs on this resource's attributes, calculations, and aggregates."
                                ),
                            ));
                        } else {
                            errors.push(unknown_ident_error(
                                field,
                                &ident_refs(&filterable),
                                "filter field",
                            ));
                        }
                    }
                }
                crate::define::ast::PreparationSpec::Limit(_)
                | crate::define::ast::PreparationSpec::Offset(_) => {}
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
                    errors.push(slot_error(
                        field,
                        "attribute",
                        &candidates,
                        &names,
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
                errors.push(slot_error(
                    key,
                    "attribute",
                    &attr_names,
                    &names,
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
        if is_uuid(inner) || is_string(inner) || is_integer(inner) || is_bool(inner) {
            errors.push(Error::new_spanned(
                &attr.ty,
                "`[enum]` requires a type that implements `AshEnum`, not a builtin scalar",
            ));
        }
    }
}

fn lint_warning(span: proc_macro2::Span, note: &str) -> proc_macro2::TokenStream {
    quote_spanned! { span =>
        const _: fn() = {
            fn __ash_lint() {
                #[deprecated(note = #note)]
                fn __ash_note() {}
                __ash_note();
            }
            __ash_lint
        };
    }
}

fn mentions_ident(tokens: proc_macro2::TokenStream, name: &Ident) -> bool {
    tokens.into_iter().any(|tt| match tt {
        proc_macro2::TokenTree::Ident(id) => id == *name,
        proc_macro2::TokenTree::Group(g) => mentions_ident(g.stream(), name),
        _ => false,
    })
}

fn lint_semantic(def: &ResourceDefinition) -> Vec<proc_macro2::TokenStream> {
    use crate::define::ast::ChangeSpec;
    let mut warnings = Vec::new();
    for action in &def.actions {
        // Only changes that write unconditionally; `set_new` and `set_from_arg` keep caller input.
        let overwrites = |name: &Ident| {
            action.changes.iter().find_map(|chg| match chg {
                ChangeSpec::Set { field, .. } if field == name => Some("set"),
                ChangeSpec::RelateActor { field } if field == name => Some("relate_actor"),
                _ => None,
            })
        };
        for acc in &action.accept {
            if let Some(change) = overwrites(&acc.name) {
                let note = format!(
                    "`accept [{}]` is overwritten by `change {change}({})` on this action",
                    acc.name, acc.name
                );
                warnings.push(lint_warning(acc.name.span(), &note));
            }
        }
        for val in &action.validations {
            if let ValidationSpec::Present { field } = val
                && let Some(attr) = def.attributes.iter().find(|a| a.ident == *field)
                && option_inner(&attr.ty).is_none()
                && !is_string(&attr.ty)
            {
                let note = format!(
                    "`present({field})` is always true because `{field}` is not `Option`"
                );
                warnings.push(lint_warning(field.span(), &note));
            }
        }
        // Opaque changes, validations, and generic runners can read any argument.
        let has_opaque_consumer = action.kind == ActionKind::Generic
            || action.changes.iter().any(|chg| {
                matches!(
                    chg,
                    ChangeSpec::BeforeAction(_)
                        | ChangeSpec::AfterAction(_)
                        | ChangeSpec::AfterTransaction(_)
                        | ChangeSpec::Custom(_)
                        | ChangeSpec::Func(_)
                )
            })
            || action
                .validations
                .iter()
                .any(|val| matches!(val, ValidationSpec::Custom(_) | ValidationSpec::Func(_)));
        if action.run_expr.is_some() || has_opaque_consumer {
            continue;
        }
        for arg in &action.arguments {
            let name = arg.name.to_string();
            let used_in_change = action.changes.iter().any(|chg| match chg {
                ChangeSpec::SetFromArg { argument, .. } => argument == &arg.name,
                ChangeSpec::ManageRelationship { relationship, .. } => relationship == &arg.name,
                ChangeSpec::Set { value, .. } | ChangeSpec::SetNew { field: _, value } => {
                    mentions_ident(quote!(#value), &arg.name)
                }
                _ => false,
            });
            let used_in_validate = action.validations.iter().any(|val| match val {
                ValidationSpec::Present { field }
                | ValidationSpec::StringLength { field, .. }
                | ValidationSpec::OneOf { field, .. }
                | ValidationSpec::Numericality { field, .. } => field == &arg.name,
                _ => false,
            });
            if !used_in_change && !used_in_validate {
                let note = format!(
                    "argument `{name}` is never used in `change`, `validate`, or `run`"
                );
                warnings.push(lint_warning(arg.name.span(), &note));
            }
        }
    }
    warnings
}

pub fn validate(def: &mut ResourceDefinition) -> Vec<Error> {
    let mut errors = Vec::new();
    check_name_collisions(def, &mut errors);
    validate_attributes(def, &mut errors);
    validate_actions(def, &mut errors);
    validate_cross_section(def, &mut errors);
    lint_uncovered_actions(def, &mut errors);
    let extra_warnings = lint_semantic(def);
    def.warnings.extend(extra_warnings);
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
    fn test_accept_relationship_names_the_wrong_slot() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                    author_id: Uuid;
                }
                relationships {
                    belongs_to author: User [fk: author_id];
                    has_many comments: Comment;
                }
                actions {
                    create open {
                        accept [comments];
                    }
                }
            }
        });
        assert!(
            msg.contains("`comments` is a relationship, not an attribute"),
            "got: {msg}"
        );
        assert!(msg.contains("change manage"), "got: {msg}");
    }

    #[test]
    fn test_set_calculation_names_the_wrong_slot() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                calculations {
                    title_length: Option<i64> = string_length(title);
                }
                actions {
                    create open {
                        change set(title_length = 1);
                    }
                }
            }
        });
        assert!(
            msg.contains("`title_length` is a calculation, not an attribute"),
            "got: {msg}"
        );
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
    fn test_filter_preparation_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                archived: bool;
            }
            actions {
                read read {
                    primary;
                    prepare filter(archved == false);
                }
            }
        }});
        assert!(msg.contains("Did you mean `archived`?"), "got: {msg}");
        assert!(msg.contains("filter field"), "got: {msg}");
    }

    #[test]
    fn test_filter_preparation_relationship_is_wrong_slot() {
        let msg = validate_err_msg(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                    author_id: Uuid;
                }
                relationships {
                    belongs_to author: User;
                    has_many comments: Comment;
                }
                actions {
                    read read {
                        primary;
                        prepare filter(comments == true);
                    }
                }
            }
        });
        assert!(msg.contains("relationship, not a filter field"), "got: {msg}");
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
    fn test_optimistic_lock_unknown_attribute_suggests_did_you_mean() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                version: i64 [version];
            }
            optimistic_lock versoin;
            actions {
                read read { primary; }
            }
        }});
        assert!(msg.contains("Did you mean `version`?"), "got: {msg}");
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
    fn test_calculation_string_length_on_integer_fails() {
        let msg = validate_err_msg(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                count: i64;
            }
            calculations {
                count_len: i64 = string_length(count);
            }
        }});
        assert!(
            msg.contains("requires a string field") && msg.contains("i64"),
            "got: {msg}"
        );
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
        let def = parse_def(quote! {
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
            def.actions[0].prepare_kw.is_some(),
            "kind-gate errors should underline `prepare`, not the action name"
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

    fn lint_notes(label: &str, tokens: proc_macro2::TokenStream) -> String {
        let mut def = parse_def(tokens);
        let errors = validate(&mut def);
        assert!(errors.is_empty(), "{label}: unexpected errors: {errors:?}");
        def.warnings.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn test_present_lint_warns_only_when_the_value_cannot_be_blank() {
        let cases = [
            ("i64", quote!(field: i64;), true),
            ("bool", quote!(field: bool;), true),
            ("Uuid", quote!(field: Uuid;), true),
            ("enum", quote!(field: Status [enum];), true),
            ("String rejects blank", quote!(field: String;), false),
            ("Option<String>", quote!(field: Option<String>;), false),
            ("Option<i64>", quote!(field: Option<i64>;), false),
        ];
        for (label, attribute, expect_warning) in cases {
            let notes = lint_notes(label, quote! {
                TestResource {
                    attributes {
                        id: Uuid [pk];
                        #attribute
                    }
                    actions {
                        create open {
                            accept [field];
                            validate present(field);
                        }
                    }
                }
            });
            assert_eq!(
                notes.contains("`present(field)` is always true"),
                expect_warning,
                "{label}: {notes}"
            );
        }
    }

    #[test]
    fn test_accept_lint_warns_only_on_unconditional_overwrites() {
        let cases = [
            ("set", quote!(status), quote!(change set(status = "open");), Some("set")),
            (
                "relate_actor",
                quote!(owner_id),
                quote!(change relate_actor(owner_id);),
                Some("relate_actor"),
            ),
            ("set_new keeps input", quote!(status), quote!(change set_new(status = "open");), None),
            (
                "set_from_arg keeps input when arg is absent",
                quote!(status),
                quote!(argument next: Option<String>; change set_from_arg(status, next);),
                None,
            ),
            ("set on another field", quote!(status), quote!(change set(note = "x");), None),
        ];
        for (label, accepted, body, expected_change) in cases {
            let notes = lint_notes(label, quote! {
                TestResource {
                    attributes {
                        id: Uuid [pk];
                        status: String;
                        note: String;
                        owner_id: Uuid;
                    }
                    actions {
                        create open {
                            accept [#accepted];
                            #body
                        }
                    }
                }
            });
            match expected_change {
                Some(change) => assert!(
                    notes.contains(&format!("is overwritten by `change {change}({accepted})`")),
                    "{label}: {notes}"
                ),
                None => assert!(!notes.contains("overwritten"), "{label}: {notes}"),
            }
        }
    }

    #[test]
    fn test_unused_argument_lint_covers_every_consumer() {
        let cases = [
            ("unused", quote!(create open { argument reason: String; }), true),
            (
                "only a string literal mentions it",
                quote!(create open { argument reason: String; change set(status = "reason"); }),
                true,
            ),
            (
                "unused beside a declarative change",
                quote!(create open { argument reason: String; change set(status = "x"); }),
                true,
            ),
            (
                "set_from_arg",
                quote!(create open { argument reason: String; change set_from_arg(status, reason); }),
                false,
            ),
            (
                "set value expression",
                quote!(create open { argument reason: String; change set(status = reason); }),
                false,
            ),
            (
                "manage_relationship",
                quote!(create open { argument items: Vec<FieldMap>; change manage_relationship(items, create); }),
                false,
            ),
            (
                "present",
                quote!(create open { argument reason: String; validate present(reason); }),
                false,
            ),
            (
                "string_length",
                quote!(create open { argument reason: String; validate string_length(reason, min: 1); }),
                false,
            ),
            (
                "one_of",
                quote!(create open { argument reason: String; validate one_of(reason, ["a", "b"]); }),
                false,
            ),
            (
                "numericality",
                quote!(create open { argument count: i64; validate numericality(count, min: 1); }),
                false,
            ),
            (
                "custom change",
                quote!(create open { argument reason: String; change custom(&Hash); }),
                false,
            ),
            (
                "func change",
                quote!(create open { argument reason: String; change func(hash); }),
                false,
            ),
            (
                "before_action",
                quote!(create open { argument reason: String; change before_action(hook); }),
                false,
            ),
            (
                "after_action",
                quote!(create open { argument reason: String; change after_action(hook); }),
                false,
            ),
            (
                "after_transaction",
                quote!(create open { argument reason: String; change after_transaction(hook); }),
                false,
            ),
            (
                "custom validation",
                quote!(create open { argument reason: String; validate custom(&Check); }),
                false,
            ),
            (
                "func validation",
                quote!(create open { argument reason: String; validate func(check); }),
                false,
            ),
            (
                "generic with run",
                quote!(generic ping, bool { argument reason: String; run |_input| async move { Ok(true) }; }),
                false,
            ),
            (
                "generic with runner supplied at call time",
                quote!(generic ping, bool { argument reason: String; }),
                false,
            ),
        ];
        for (label, action, expect_warning) in cases {
            let notes = lint_notes(label, quote! {
                TestResource {
                    attributes {
                        id: Uuid [pk];
                        status: String;
                    }
                    relationships {
                        has_many items: Item [fk: parent_id];
                    }
                    actions {
                        #action
                    }
                }
            });
            assert_eq!(notes.contains("is never used"), expect_warning, "{label}: {notes}");
        }
    }
}
