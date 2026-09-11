use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::Ident;

use crate::ast_helpers::{option_inner, pascal_case};
use crate::define::ast::{
    ActionKind, AggregateKindSpec, CalculationExprSpec, ChangeSpec, PolicyCheckExpr,
    PolicyEffectSpec, PolicyWhenSpec, PreparationSpec, RelType, ResourceDefinition, ValidationSpec,
};
use quote::format_ident;
use syn::spanned::Spanned;

fn record_has_field(def: &ResourceDefinition, name: &Ident) -> bool {
    def.attributes.iter().any(|a| a.ident == *name)
        || def.calculations.iter().any(|c| c.ident == *name)
        || def.aggregates.iter().any(|a| a.ident == *name)
        || def.relationships.iter().any(|r| r.ident == *name)
}

pub fn expand_ide_probe(def: &ResourceDefinition) -> TokenStream {
    let resource = &def.resource;

    let mut action_probes = Vec::new();

    for action in &def.actions {
        let mut field_probes = Vec::new();

        // 1. Action arguments declared as local variables in probe scope.
        // Using `loop {}` allows type coercion from `!` without calling unwrap/panic.
        // Bind owned values so `__ash_assert_assignable(..., &#argument)` checks `Arg: Into<Target>`.
        for arg in &action.arguments {
            let arg_name = &arg.name;
            let arg_ty = &arg.ty;
            field_probes.push(quote_spanned! { arg_name.span() =>
                let #arg_name: #arg_ty = loop {};
                let _ = &#arg_name;
            });
        }

        // 2. Accept fields: resource attributes on CRUD actions, locals on generic actions.
        for acc in &action.accept {
            let name = &acc.name;
            if action.kind == ActionKind::Generic {
                let ty = &acc.ty;
                field_probes.push(quote_spanned! { name.span() =>
                    let #name: #ty = loop {};
                    let _ = &#name;
                });
            } else {
                field_probes.push(quote_spanned! { name.span() =>
                    let _ = &__ash_record.#name;
                });
            }
        }

        // 3. Validation fields: if they match an attribute on the struct, probe &__ash_record.#field.
        // If they match an action argument, probe that local variable.
        for val in &action.validations {
            match val {
                ValidationSpec::Present { field }
                | ValidationSpec::StringLength { field, .. }
                | ValidationSpec::OneOf { field, .. }
                | ValidationSpec::Numericality { field, .. } => {
                    let is_arg = action.arguments.iter().any(|a| a.name == *field)
                        || (action.kind == ActionKind::Generic
                            && action.accept.iter().any(|a| a.name == *field));
                    if is_arg {
                        field_probes.push(quote_spanned! { field.span() =>
                            let _ = &#field;
                        });
                    } else {
                        field_probes.push(quote_spanned! { field.span() =>
                            let _ = &__ash_record.#field;
                        });
                    }
                }
                _ => {}
            }
        }

        // 4. Change fields
        for chg in &action.changes {
            match chg {
                ChangeSpec::Set { field, value } => {
                    let attr = def.attributes.iter().find(|a| a.ident == *field);
                    match attr {
                        Some(attr) if option_inner(&attr.ty).is_some() => {
                            field_probes.push(quote_spanned! { value.span() =>
                                __ash_assert_assignable_optional(&__ash_record.#field, &#value);
                            });
                        }
                        Some(_) => {
                            field_probes.push(quote_spanned! { value.span() =>
                                __ash_assert_assignable(&__ash_record.#field, &#value);
                            });
                        }
                        _ => {
                            field_probes.push(quote_spanned! { field.span() =>
                                let _ = &__ash_record.#field;
                            });
                        }
                    }
                }
                ChangeSpec::SetNew { field, value } => {
                    field_probes.push(quote_spanned! { value.span() =>
                        __ash_assert_assignable(&__ash_record.#field, &#value);
                    });
                }
                ChangeSpec::SetFromArg { field, argument } => {
                    field_probes.push(quote_spanned! { argument.span() =>
                        __ash_assert_assignable(&__ash_record.#field, &#argument);
                    });
                }
                ChangeSpec::RelateActor { field } => {
                    field_probes.push(quote_spanned! { field.span() =>
                        let _ = &__ash_record.#field;
                    });
                }
                ChangeSpec::ManageRelationship { relationship, .. } => {
                    field_probes.push(quote_spanned! { relationship.span() =>
                        let _ = &__ash_record.#relationship;
                    });
                }
                _ => {}
            }
        }

        // 5. Preparation fields (sort)
        for prep in &action.preparations {
            if let PreparationSpec::Sort { field, .. } = prep {
                field_probes.push(quote_spanned! { field.span() =>
                    let _ = &__ash_record.#field;
                });
            }
        }

        if let Some(run_expr) = &action.run_expr {
            let input_struct_name =
                format_ident!("{}{}Input", resource, pascal_case(&action.name.to_string()));
            let returns_ty = action
                .returns
                .clone()
                .unwrap_or_else(|| syn::parse_quote!(()));
            field_probes.push(quote! {
                if false {
                    fn __ash_probe_run<'a, F, Fut>(_: F)
                    where
                        F: ::std::ops::FnOnce(
                            #input_struct_name<'a, ::ash_memory::Memory>,
                        ) -> Fut,
                        Fut: ::std::future::Future<Output = ::ash_core::Result<#returns_ty>>,
                    {
                    }
                    __ash_probe_run(#run_expr);
                }
            });
        }

        action_probes.push(quote! {
            {
                #(#field_probes)*
            }
        });
    }

    let section_probes = expand_cross_section_probes(def);
    let needs_ash_type = def.attributes.iter().any(|a| a.uses_ash_type_storage());
    let needs_enum = def.attributes.iter().any(|a| a.is_enum);
    let ash_type_helper = if needs_ash_type {
        quote! {
            #[allow(dead_code)]
            fn __ash_assert_ash_type<T: ::ash_core::AshType>() {}
        }
    } else {
        quote! {}
    };
    let enum_helper = if needs_enum {
        quote! {
            #[allow(dead_code)]
            fn __ash_assert_enum<T: ::ash_core::AshEnum>() {}
        }
    } else {
        quote! {}
    };

    quote! {
        #[doc(hidden)]
        const _: () = {
            #[allow(
                dead_code,
                unused_variables,
                unused_imports,
                non_snake_case,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_assert_assignable<Target, Value: ::std::convert::Into<Target>>(
                _: &Target,
                _: &Value,
            ) {
            }

            #[allow(
                dead_code,
                unused_variables,
                unused_imports,
                non_snake_case,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_assert_assignable_optional<Inner, Value: ::std::convert::Into<Inner>>(
                _: &::std::option::Option<Inner>,
                _: &Value,
            ) {
            }

            #[allow(
                dead_code,
                unused_variables,
                unused_imports,
                non_snake_case,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_ide_typecheck(__ash_record: &#resource) {
                #[allow(dead_code)]
                fn __ash_assert_resource<T: ::ash_core::Resource>() {}
                #ash_type_helper
                #enum_helper

                if false {
                    #(#action_probes)*
                    #(#section_probes)*
                }
            }
        };
    }
}

fn expand_cross_section_probes(def: &ResourceDefinition) -> Vec<TokenStream> {
    let mut probes = Vec::new();
    let resource = &def.resource;

    for attr in &def.attributes {
        if !attr.uses_ash_type_storage() {
            continue;
        }
        let inner = option_inner(&attr.ty).unwrap_or(&attr.ty);
        probes.push(quote_spanned! { inner.span() =>
            __ash_assert_ash_type::<#inner>();
        });
        if attr.is_enum {
            probes.push(quote_spanned! { inner.span() =>
                __ash_assert_enum::<#inner>();
            });
        }
    }

    for rel in &def.relationships {
        let dest = &rel.dest;
        probes.push(quote_spanned! { dest.span() =>
            __ash_assert_resource::<#dest>();
        });
    }

    for rel in &def.relationships {
        if let Some(fk) = &rel.fk {
            if record_has_field(def, fk) {
                probes.push(quote_spanned! { fk.span() =>
                    let _ = &__ash_record.#fk;
                });
            }
            if matches!(rel.kind, RelType::HasMany | RelType::HasOne) {
                probes.push(dest_field_probe(&rel.dest, fk));
            }
        }
    }

    for agg in &def.aggregates {
        let rel = &agg.relationship;
        probes.push(quote_spanned! { rel.span() =>
            let _ = &__ash_record.#rel;
        });
        match &agg.kind {
            AggregateKindSpec::First { field } | AggregateKindSpec::Sum { field } => {
                if let Some(r) = def.relationships.iter().find(|r| r.ident == *rel) {
                    probes.push(dest_field_probe(&r.dest, field));
                }
            }
            AggregateKindSpec::Count | AggregateKindSpec::Exists => {}
        }
    }

    for identity in &def.identities {
        for key in &identity.keys {
            probes.push(quote_spanned! { key.span() =>
                let _ = &__ash_record.#key;
            });
        }
    }

    for fp in &def.field_policies {
        let field = &fp.field;
        probes.push(quote_spanned! { field.span() =>
            let _ = &__ash_record.#field;
        });
        for check in &fp.checks {
            collect_policy_check_probes(def, resource, check_ref(check), &mut probes);
        }
    }

    for pol in &def.policies {
        for when in &pol.whens {
            if let PolicyWhenSpec::ActionName(name) = when {
                let method = quote_spanned! { name.span() => #name };
                let act = def.actions.iter().find(|a| a.name == *name);
                let probe = match act.map(|a| a.kind) {
                    Some(ActionKind::Read) => quote_spanned! { name.span() =>
                        if false {
                            let _ = #resource::query::<::ash_memory::Memory>;
                        }
                    },
                    Some(ActionKind::Update) | Some(ActionKind::Destroy) => quote! {
                        if false {
                            let ctx: &::ash_core::Context<::ash_memory::Memory> = loop {};
                            let rec: #resource = loop {};
                            let _ = #resource::#method(ctx, rec);
                        }
                    },
                    _ => quote! {
                        if false {
                            let ctx: &::ash_core::Context<::ash_memory::Memory> = loop {};
                            let _ = #resource::#method(ctx);
                        }
                    },
                };
                probes.push(probe);
            }
        }
        for check in &pol.checks {
            collect_policy_check_probes(def, resource, check_ref(check), &mut probes);
        }
    }

    for calc in &def.calculations {
        let mut calc_probes = Vec::new();
        for arg in &calc.arguments {
            let name = &arg.name;
            let ty = &arg.ty;
            calc_probes.push(quote_spanned! { name.span() =>
                let #name: #ty = loop {};
                let _ = &#name;
            });
        }
        collect_calc_field_probes(&calc.expr, &mut calc_probes);
        probes.push(quote! {
            {
                #(#calc_probes)*
            }
        });
    }

    probes
}

fn check_ref(effect: &PolicyEffectSpec) -> &PolicyCheckExpr {
    match effect {
        PolicyEffectSpec::AuthorizeIf(c)
        | PolicyEffectSpec::AuthorizeUnless(c)
        | PolicyEffectSpec::ForbidIf(c)
        | PolicyEffectSpec::ForbidUnless(c) => c,
    }
}

fn collect_policy_check_probes(
    def: &ResourceDefinition,
    _resource: &Ident,
    check: &PolicyCheckExpr,
    probes: &mut Vec<TokenStream>,
) {
    match check {
        PolicyCheckExpr::ActorAttributeEquals { attr, value } => {
            if let Some(actor) = &def.actor
                && let Some(field) = actor.fields.iter().find(|f| f.name == *attr)
            {
                let ty = &field.ty;
                probes.push(quote_spanned! { value.span() =>
                    let __ash_actor_field: #ty = loop {};
                    __ash_assert_assignable(&__ash_actor_field, &#value);
                });
            }
        }
        PolicyCheckExpr::And(parts) | PolicyCheckExpr::Or(parts) => {
            for part in parts {
                collect_policy_check_probes(def, _resource, part, probes);
            }
        }
        _ => {}
    }
}

fn collect_calc_field_probes(expr: &CalculationExprSpec, probes: &mut Vec<TokenStream>) {
    match expr {
        CalculationExprSpec::Field(field) => {
            probes.push(quote_spanned! { field.span() =>
                let _ = &__ash_record.#field;
            });
        }
        CalculationExprSpec::Arg(ident) => {
            probes.push(quote_spanned! { ident.span() =>
                let _ = &#ident;
            });
        }
        CalculationExprSpec::StringLength(field) => {
            probes.push(quote_spanned! { field.span() =>
                let _ = &__ash_record.#field;
            });
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
            collect_calc_field_probes(left, probes);
            collect_calc_field_probes(right, probes);
        }
        CalculationExprSpec::Concat(parts) | CalculationExprSpec::Coalesce(parts) => {
            for part in parts {
                collect_calc_field_probes(part, probes);
            }
        }
        CalculationExprSpec::Lower(inner)
        | CalculationExprSpec::Upper(inner)
        | CalculationExprSpec::Length(inner) => {
            collect_calc_field_probes(inner, probes);
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            collect_calc_field_probes(cond, probes);
            collect_calc_field_probes(then_expr, probes);
            collect_calc_field_probes(else_expr, probes);
        }
        CalculationExprSpec::LitInt(_)
        | CalculationExprSpec::LitString(_)
        | CalculationExprSpec::LitBool(_)
        | CalculationExprSpec::Null
        | CalculationExprSpec::Custom(_) => {}
    }
}

fn dest_field_probe(dest: &Ident, field: &Ident) -> TokenStream {
    let access = quote_spanned! { field.span() =>
        let _ = &__dest_stub.#field;
    };
    quote! {
        {
            #[allow(dead_code, unreachable_code, clippy::all, clippy::pedantic)]
            fn __ash_probe_dest<Dest: ::ash_core::Resource>(__dest: &Dest) {
                let _ = __dest;
            }
            let __dest_stub: &#dest = loop {};
            __ash_probe_dest(__dest_stub);
            #access
        }
    }
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

    #[test]
    fn test_probe_emits_assignable_assert_for_set() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                status: String;
            }
            actions {
                create open {
                    change set(status = "open");
                }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(
            out.contains("__ash_assert_assignable"),
            "missing helper: {out}"
        );
        assert!(out.contains("status"), "missing field: {out}");
    }

    #[test]
    fn test_probe_emits_assignable_assert_for_set_from_arg() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                reason: String;
            }
            actions {
                create open {
                    argument reason_input: String;
                    change set_from_arg(reason, reason_input);
                }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(
            out.contains("__ash_assert_assignable"),
            "missing helper: {out}"
        );
        assert!(out.contains("reason_input"), "missing arg: {out}");
    }

    #[test]
    fn test_probe_covers_relationships_aggregates_identities_and_calculations() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                author_id: Uuid;
                title: String;
            }
            relationships {
                belongs_to author: User [fk: author_id];
                has_many comments: Comment;
            }
            aggregates {
                comment_count: i64 = count(comments);
                first_title: Option<String> = first(comments, title);
            }
            identities {
                identity by_title: [title];
            }
            field_policies {
                field title {
                    authorize_if always;
                }
            }
            calculations {
                titled(prefix: String): String = concat(arg(prefix), title);
            }
            actions {
                read read { primary; }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("author_id"), "missing fk probe: {out}");
        assert!(out.contains("comments"), "missing aggregate rel: {out}");
        assert!(out.contains("title"), "missing identity/field: {out}");
        assert!(out.contains("prefix"), "missing calc arg: {out}");
        assert!(
            out.contains("__ash_assert_resource"),
            "missing relationship resource probe: {out}"
        );
        assert!(out.contains("User"), "missing dest type: {out}");
        assert!(out.contains("Comment"), "missing dest type: {out}");
        assert!(
            out.contains("__ash_probe_dest"),
            "missing destination probe: {out}"
        );
        assert!(out.contains("__dest_stub"), "missing dest stub: {out}");
    }

    #[test]
    fn test_probe_emits_dest_fk_for_has_many() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            relationships {
                has_many comments: Comment [fk: ticket_id];
            }
            actions {
                read read { primary; }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("ticket_id"), "missing dest fk: {out}");
        assert!(out.contains("Comment"), "missing dest type: {out}");
        assert!(
            out.contains("__ash_probe_dest"),
            "missing dest fk probe: {out}"
        );
    }

    #[test]
    fn test_probe_covers_generic_action_inputs_and_run_expr() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            actions {
                generic summarize {
                    argument notes: String;
                    accept {
                        extra: String,
                    };
                    returns String;
                    run |input| async move { Ok(input.notes) };
                }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("notes"), "missing generic arg: {out}");
        assert!(out.contains("extra"), "missing generic accept: {out}");
        assert!(
            out.contains("__ash_probe_run"),
            "missing generic run probe: {out}"
        );
        assert!(
            out.contains("TestResourceSummarizeInput"),
            "missing generic input type: {out}"
        );
    }
}
