use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::{Ident, Type};

use crate::ast_helpers::{is_bool, is_i64, is_string, is_uuid, option_inner};
use crate::define::ast::{
    AggregateKindSpec, CalculationExprSpec, ChangeSpec, PreparationSpec, ResourceDefinition,
    ValidationSpec,
};

fn record_has_field(def: &ResourceDefinition, name: &Ident) -> bool {
    def.attributes.iter().any(|a| a.ident == *name)
        || def.calculations.iter().any(|c| c.ident == *name)
        || def.aggregates.iter().any(|a| a.ident == *name)
        || def.relationships.iter().any(|r| r.ident == *name)
}

fn is_plain_assignable_inner(ty: &Type) -> bool {
    is_string(ty) || is_i64(ty) || is_bool(ty) || is_uuid(ty)
}

pub fn expand_ide_probe(def: &ResourceDefinition) -> TokenStream {
    let resource = &def.resource;

    let mut action_probes = Vec::new();

    for action in &def.actions {
        if action.kind == crate::define::ast::ActionKind::Generic {
            continue;
        }

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

        // 2. Accept fields (inferred bracket-form or explicit brace-form) reference struct fields
        for acc in &action.accept {
            let name = &acc.name;
            field_probes.push(quote_spanned! { name.span() =>
                let _ = &__ash_record.#name;
            });
        }

        // 3. Validation fields: if they match an attribute on the struct, probe &__ash_record.#field.
        // If they match an action argument, probe that local variable.
        for val in &action.validations {
            match val {
                ValidationSpec::Present { field }
                | ValidationSpec::StringLength { field, .. }
                | ValidationSpec::OneOf { field, .. }
                | ValidationSpec::Numericality { field, .. } => {
                    let is_arg = action.arguments.iter().any(|a| a.name == *field);
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
                        Some(attr)
                            if option_inner(&attr.ty).is_some_and(is_plain_assignable_inner) =>
                        {
                            field_probes.push(quote_spanned! { value.span() =>
                                __ash_assert_assignable_optional(&__ash_record.#field, &#value);
                            });
                        }
                        Some(attr) if is_plain_assignable_inner(&attr.ty) => {
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
                ChangeSpec::SetFromArg { field, argument } => {
                    field_probes.push(quote_spanned! { argument.span() =>
                        __ash_assert_assignable(&__ash_record.#field, &#argument);
                    });
                }
                ChangeSpec::SetNew { field, .. } | ChangeSpec::RelateActor { field } => {
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

        action_probes.push(quote! {
            {
                #(#field_probes)*
            }
        });
    }

    let section_probes = expand_cross_section_probes(def);

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

    for rel in &def.relationships {
        let dest = &rel.dest;
        probes.push(quote_spanned! { dest.span() =>
            __ash_assert_resource::<#dest>();
        });
    }

    for rel in &def.relationships {
        if let Some(fk_name) = &rel.fk {
            let span = rel.fk_span.unwrap_or_else(proc_macro2::Span::call_site);
            if let Some(fk_ident) = ident_with_span(fk_name, span)
                && record_has_field(def, &fk_ident)
            {
                probes.push(quote_spanned! { span =>
                    let _ = &__ash_record.#fk_ident;
                });
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
                if record_has_field(def, field) {
                    probes.push(quote_spanned! { field.span() =>
                        let _ = &__ash_record.#field;
                    });
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

fn collect_calc_field_probes(expr: &CalculationExprSpec, probes: &mut Vec<TokenStream>) {
    match expr {
        CalculationExprSpec::Field(field) => {
            probes.push(quote_spanned! { field.span() =>
                let _ = &__ash_record.#field;
            });
        }
        CalculationExprSpec::Arg(name) => {
            if let Some(ident) = ident_with_span(name, proc_macro2::Span::call_site()) {
                probes.push(quote! {
                    let _ = &#ident;
                });
            }
        }
        CalculationExprSpec::StringLength(name) => {
            if let Some(field) = ident_with_span(name, proc_macro2::Span::call_site()) {
                probes.push(quote! {
                    let _ = &__ash_record.#field;
                });
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

fn ident_with_span(name: &str, span: proc_macro2::Span) -> Option<syn::Ident> {
    let parsed: syn::Ident = syn::parse_str(name).ok()?;
    Some(syn::Ident::new(&parsed.to_string(), span))
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
            resource TestResource;
            attributes {
                id: Uuid [pk],
                status: String,
            }
            actions {
                create open {
                    change set(status = "open");
                }
            }
        });
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
            resource TestResource;
            attributes {
                id: Uuid [pk],
                reason: String,
            }
            actions {
                create open {
                    argument reason_input: String;
                    change set_from_arg(reason, reason_input);
                }
            }
        });
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
            resource TestResource;
            attributes {
                id: Uuid [pk],
                author_id: Uuid,
                title: String,
            }
            relationships {
                belongs_to author: User [fk: "author_id"];
                has_many comments: Vec<Comment>;
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
        });
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
    }
}
