use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::Ident;

use crate::ast_helpers::{option_inner, pascal_case, snake_case};
use crate::define::ast::{
    ActionKind, AggregateFilterSpec, AggregateKindSpec, CalculationExprSpec, ChangeSpec,
    PolicyCheckExpr, PolicyEffectSpec, PolicyWhenSpec, PreparationSpec, RelType,
    ResourceDefinition, ValidationSpec,
};
use quote::format_ident;
use syn::spanned::Spanned;

fn record_has_field(def: &ResourceDefinition, name: &Ident) -> bool {
    def.attributes.iter().any(|a| a.ident == *name)
        || def.calculations.iter().any(|c| c.ident == *name)
        || def.aggregates.iter().any(|a| a.ident == *name)
        || def.relationships.iter().any(|r| r.ident == *name)
}

fn ns_field_probe(target: &Ident, name: &Ident) -> TokenStream {
    quote_spanned! { name.span() =>
        let _ = &#target.#name;
    }
}

/// Completion namespaces so `accept [...]`, relationship slots, and
/// `policy action(...)` offer only the names valid in that slot — not every
/// method and field on the resource struct.
fn expand_completion_namespaces(def: &ResourceDefinition) -> TokenStream {
    let attr_fields = def.attributes.iter().map(|a| {
        let id = &a.ident;
        let ty = &a.ty;
        quote! { pub #id: #ty }
    });
    let rel_fields = def.relationships.iter().map(|r| {
        let id = &r.ident;
        let dest = &r.dest;
        quote! { pub #id: #dest }
    });
    let action_fields = def.actions.iter().map(|a| {
        let id = &a.name;
        quote! { pub #id: () }
    });
    let actor_fields = def.actor.iter().flat_map(|actor| {
        actor.fields.iter().map(|f| {
            let id = &f.name;
            let ty = &f.ty;
            quote! { pub #id: #ty }
        })
    });
    let actor_struct = if def.actor.is_some() {
        quote! {
            #[allow(dead_code, non_camel_case_types)]
            struct __AshActorFields {
                #(#actor_fields,)*
            }
        }
    } else {
        quote! {}
    };

    quote! {
        #[allow(dead_code, non_camel_case_types)]
        struct __AshAcceptFields {
            #(#attr_fields,)*
        }
        #[allow(dead_code, non_camel_case_types)]
        struct __AshRelFields {
            #(#rel_fields,)*
        }
        #[allow(dead_code, non_camel_case_types)]
        struct __AshActionNames {
            #(#action_fields,)*
        }
        #actor_struct
    }
}

pub fn expand_ide_probe(def: &ResourceDefinition) -> TokenStream {
    let resource = &def.resource;
    let namespaces = expand_completion_namespaces(def);
    let has_actor = def.actor.is_some();
    let actor_bind = if has_actor {
        quote! {
            let __ash_actor: &__AshActorFields = loop {};
        }
    } else {
        quote! {}
    };

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

        // 2. Accept fields: attribute namespace on CRUD, locals on generic actions.
        for acc in &action.accept {
            let name = &acc.name;
            if action.kind == ActionKind::Generic {
                let ty = &acc.ty;
                field_probes.push(quote_spanned! { name.span() =>
                    let #name: #ty = loop {};
                    let _ = &#name;
                });
            } else {
                field_probes.push(ns_field_probe(
                    &Ident::new("__ash_accept", name.span()),
                    name,
                ));
            }
        }
        if action.accept.is_empty()
            && let Some(span) = action.accept_span
        {
            let dummy = Ident::new("__ash_slot", span);
            field_probes.push(quote! {
                #[cfg(rust_analyzer)]
                {
                    let _ = &__ash_accept.#dummy;
                }
            });
        }

        // 3. Validation fields: attribute namespace, or the argument local.
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
                        field_probes.push(ns_field_probe(
                            &Ident::new("__ash_accept", field.span()),
                            field,
                        ));
                    }
                }
                ValidationSpec::Custom(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: &'static dyn ::ash_core::CustomValidation = #expr;
                    });
                }
                ValidationSpec::Func(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: fn(&::ash_core::ValidationContext<'_>) -> ::ash_core::Result<()> =
                            #expr;
                    });
                }
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
                    field_probes.push(ns_field_probe(
                        &Ident::new("__ash_rel", relationship.span()),
                        relationship,
                    ));
                }
                ChangeSpec::BeforeAction(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: ::ash_core::BeforeActionFn = #expr;
                    });
                }
                ChangeSpec::AfterAction(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: ::ash_core::AfterActionFn = #expr;
                    });
                }
                ChangeSpec::AfterTransaction(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: ::ash_core::AfterTransactionFn = #expr;
                    });
                }
                ChangeSpec::Custom(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: &'static dyn ::ash_core::CustomChange = #expr;
                    });
                }
                ChangeSpec::Func(expr) => {
                    field_probes.push(quote_spanned! { expr.span() =>
                        let _: fn(&mut ::ash_core::ChangeContext<'_>) -> ::ash_core::Result<()> =
                            #expr;
                    });
                }
            }
        }

        // 5. Preparation fields (sort + filter)
        for prep in &action.preparations {
            match prep {
                PreparationSpec::Sort { field, .. } => {
                    field_probes.push(ns_field_probe(
                        &Ident::new("__ash_accept", field.span()),
                        field,
                    ));
                }
                PreparationSpec::Filter { expr } => {
                    let filter_tokens = super::actions::filter_expr_to_tokens(expr, resource);
                    field_probes.push(quote! {
                        let _: ::ash_core::Filter = #filter_tokens;
                    });
                }
                PreparationSpec::Limit(_) | PreparationSpec::Offset(_) => {}
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

    let calc_mod = format_ident!(
        "__ash_{}_calc_probe",
        snake_case(&resource.to_string())
    );
    let calc_helpers = if def.calculations.is_empty() {
        quote! {}
    } else {
        quote! {
            #[doc(hidden)]
            #[allow(dead_code, non_camel_case_types)]
            mod #calc_mod {
                pub trait __AshStrLen {
                    fn __ash_str_len(&self) -> i64;
                }
                impl __AshStrLen for ::std::string::String {
                    fn __ash_str_len(&self) -> i64 {
                        self.chars().count() as i64
                    }
                }
                impl __AshStrLen for ::std::option::Option<::std::string::String> {
                    fn __ash_str_len(&self) -> i64 {
                        self.as_ref()
                            .map(|s| s.chars().count() as i64)
                            .unwrap_or(0)
                    }
                }
                pub trait __AshStrCase {
                    type Out;
                    fn __ash_lower(self) -> Self::Out;
                    fn __ash_upper(self) -> Self::Out;
                }
                impl __AshStrCase for ::std::string::String {
                    type Out = ::std::string::String;
                    fn __ash_lower(self) -> Self::Out {
                        self.to_lowercase()
                    }
                    fn __ash_upper(self) -> Self::Out {
                        self.to_uppercase()
                    }
                }
                impl __AshStrCase for ::std::option::Option<::std::string::String> {
                    type Out = ::std::option::Option<::std::string::String>;
                    fn __ash_lower(self) -> Self::Out {
                        self.map(|s| s.to_lowercase())
                    }
                    fn __ash_upper(self) -> Self::Out {
                        self.map(|s| s.to_uppercase())
                    }
                }
                pub trait __AshCalcFits<Declared> {}
                impl<T> __AshCalcFits<T> for T {}
                impl<T> __AshCalcFits<::std::option::Option<T>> for T {}
                pub fn __ash_calc_fits<Declared, Inferred: __AshCalcFits<Declared>>(
                    _: &Declared,
                    _: &Inferred,
                ) {
                }
            }
        }
    };
    let calc_use = if def.calculations.is_empty() {
        quote! {}
    } else {
        quote! {
            use #calc_mod::{__AshCalcFits, __AshStrCase, __AshStrLen, __ash_calc_fits};
        }
    };

    quote! {
        #calc_helpers
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
                #calc_use
                #namespaces
                #[allow(dead_code)]
                fn __ash_assert_resource<T: ::ash_core::Resource>() {}
                #[allow(dead_code)]
                fn __ash_assert_store<T: ::ash_core::StoreTag>() {}
                #ash_type_helper
                #enum_helper

                if false {
                    let __ash_accept: &__AshAcceptFields = loop {};
                    let __ash_rel: &__AshRelFields = loop {};
                    let __ash_actions: &__AshActionNames = loop {};
                    #actor_bind
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
        if let Some(through) = &rel.through {
            probes.push(quote_spanned! { through.span() =>
                __ash_assert_resource::<#through>();
            });
        }
    }

    if let Some(store_ty) = &def.store {
        probes.push(quote_spanned! { store_ty.span() =>
            __ash_assert_store::<#store_ty>();
        });
    }

    for attr in &def.attributes {
        if let Some(default_fn) = &attr.default_fn {
            let ty = &attr.ty;
            probes.push(quote_spanned! { default_fn.span() =>
                let _: #ty = (#default_fn)();
            });
        }
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
        probes.push(ns_field_probe(
            &Ident::new("__ash_rel", rel.span()),
            rel,
        ));
        match &agg.kind {
            AggregateKindSpec::First { field } | AggregateKindSpec::Sum { field } => {
                if let Some(r) = def.relationships.iter().find(|r| r.ident == *rel) {
                    probes.push(dest_field_probe(&r.dest, field));
                }
            }
            AggregateKindSpec::Count | AggregateKindSpec::Exists => {}
        }
        if let Some(filter) = &agg.filter {
            let field = match filter {
                AggregateFilterSpec::Eq { field, .. } | AggregateFilterSpec::Ne { field, .. } => {
                    field
                }
            };
            if let Some(r) = def.relationships.iter().find(|r| r.ident == *rel) {
                probes.push(dest_field_probe(&r.dest, field));
            }
        }
    }

    for notifier in &def.notifiers {
        probes.push(quote_spanned! { notifier.span() =>
            let _: &'static dyn ::ash_core::Notifier = #notifier;
        });
    }

    for ext in &def.extensions {
        probes.push(quote_spanned! { ext.span() =>
            let _: &'static dyn ::ash_core::ResourceExtension = #ext;
        });
    }

    if let Some(lock) = &def.optimistic_lock {
        probes.push(ns_field_probe(
            &Ident::new("__ash_accept", lock.span()),
            lock,
        ));
    }

    for identity in &def.identities {
        for key in &identity.keys {
            probes.push(ns_field_probe(
                &Ident::new("__ash_accept", key.span()),
                key,
            ));
        }
    }

    for fp in &def.field_policies {
        let field = &fp.field;
        probes.push(ns_field_probe(
            &Ident::new("__ash_accept", field.span()),
            field,
        ));
        for check in &fp.checks {
            collect_policy_check_probes(def, resource, check_ref(check), &mut probes);
        }
    }

    for pol in &def.policies {
        for when in &pol.whens {
            if let PolicyWhenSpec::ActionName(name) = when {
                probes.push(ns_field_probe(
                    &Ident::new("__ash_actions", name.span()),
                    name,
                ));
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
        if let Some(typed) = calc_expr_typed(&calc.expr) {
            let ty = &calc.ty;
            calc_probes.push(quote! {
                let __ash_declared: #ty = loop {};
                let __ash_inferred = #typed;
                __ash_calc_fits(&__ash_declared, &__ash_inferred);
            });
        }
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

fn policy_slot_probe(def: &ResourceDefinition, field: &Ident) -> TokenStream {
    if def.relationships.iter().any(|r| r.ident == *field) {
        ns_field_probe(&Ident::new("__ash_rel", field.span()), field)
    } else {
        ns_field_probe(&Ident::new("__ash_accept", field.span()), field)
    }
}

fn collect_policy_check_probes(
    def: &ResourceDefinition,
    _resource: &Ident,
    check: &PolicyCheckExpr,
    probes: &mut Vec<TokenStream>,
) {
    match check {
        PolicyCheckExpr::RelatesToActor(field) | PolicyCheckExpr::IsNil(field) => {
            probes.push(policy_slot_probe(def, field));
        }
        PolicyCheckExpr::Eq { field, value } => {
            probes.push(policy_slot_probe(def, field));
            if let Some(attr) = def.attributes.iter().find(|a| a.ident == *field) {
                if option_inner(&attr.ty).is_some() {
                    probes.push(quote_spanned! { value.span() =>
                        __ash_assert_assignable_optional(&__ash_record.#field, &#value);
                    });
                } else {
                    probes.push(quote_spanned! { value.span() =>
                        __ash_assert_assignable(&__ash_record.#field, &#value);
                    });
                }
            }
        }
        PolicyCheckExpr::ActorAttributeEquals { attr, value } => {
            if def.actor.is_some() {
                probes.push(ns_field_probe(
                    &Ident::new("__ash_actor", attr.span()),
                    attr,
                ));
            }
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
        PolicyCheckExpr::Always | PolicyCheckExpr::ActorPresent => {}
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
        | CalculationExprSpec::Null => {}
        CalculationExprSpec::Custom(path) => {
            probes.push(quote_spanned! { path.span() =>
                let _: fn(&::ash_core::FieldMap) -> ::ash_core::Result<::ash_core::Value> = #path;
            });
        }
    }
}

fn calc_expr_typed(expr: &CalculationExprSpec) -> Option<TokenStream> {
    match expr {
        CalculationExprSpec::Field(field) => Some(quote_spanned! { field.span() =>
            __ash_record.#field.clone()
        }),
        CalculationExprSpec::Arg(ident) => Some(quote_spanned! { ident.span() =>
            #ident.clone()
        }),
        CalculationExprSpec::StringLength(field) => Some(quote_spanned! { field.span() =>
            __AshStrLen::__ash_str_len(&__ash_record.#field)
        }),
        CalculationExprSpec::LitInt(n) => Some(quote! { #n }),
        CalculationExprSpec::LitString(s) => {
            Some(quote! { ::std::string::String::from(#s) })
        }
        CalculationExprSpec::LitBool(b) => Some(quote! { #b }),
        CalculationExprSpec::Length(inner) => {
            let inner = calc_expr_typed(inner)?;
            Some(quote! { __AshStrLen::__ash_str_len(&#inner) })
        }
        CalculationExprSpec::Lower(inner) => {
            let inner = calc_expr_typed(inner)?;
            Some(quote! { __AshStrCase::__ash_lower(#inner) })
        }
        CalculationExprSpec::Upper(inner) => {
            let inner = calc_expr_typed(inner)?;
            Some(quote! { __AshStrCase::__ash_upper(#inner) })
        }
        CalculationExprSpec::Add(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) + (#right) })
        }
        CalculationExprSpec::Sub(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) - (#right) })
        }
        CalculationExprSpec::Mul(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) * (#right) })
        }
        CalculationExprSpec::Div(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) / (#right) })
        }
        CalculationExprSpec::Eq(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) == (#right) })
        }
        CalculationExprSpec::Ne(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) != (#right) })
        }
        CalculationExprSpec::Gt(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) > (#right) })
        }
        CalculationExprSpec::Gte(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) >= (#right) })
        }
        CalculationExprSpec::Lt(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) < (#right) })
        }
        CalculationExprSpec::Lte(left, right) => {
            let left = calc_expr_typed(left)?;
            let right = calc_expr_typed(right)?;
            Some(quote! { (#left) <= (#right) })
        }
        CalculationExprSpec::Concat(parts) => {
            let typed: Vec<_> = parts.iter().map(calc_expr_typed).collect::<Option<_>>()?;
            let mut typed = typed.into_iter();
            let mut acc = typed.next()?;
            for part in typed {
                acc = quote! { ::std::format!("{}{}", #acc, #part) };
            }
            Some(acc)
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            let cond = calc_expr_typed(cond)?;
            let then_expr = calc_expr_typed(then_expr)?;
            let else_expr = calc_expr_typed(else_expr)?;
            Some(quote! {
                if #cond {
                    #then_expr
                } else {
                    #else_expr
                }
            })
        }
        CalculationExprSpec::Coalesce(_)
        | CalculationExprSpec::Null
        | CalculationExprSpec::Custom(_) => None,
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
    fn test_probe_aggregate_filter_on_dest_field() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            relationships {
                has_many tickets: Ticket;
            }
            aggregates {
                open_ticket_count: Option<i64> = count(tickets, filter: status == "open");
            }
            actions {
                read read { primary; }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("status"), "missing dest filter field: {out}");
        assert!(out.contains("Ticket"), "missing dest type: {out}");
        assert!(
            out.contains("__ash_probe_dest"),
            "missing dest filter probe: {out}"
        );
    }

    #[test]
    fn test_probe_lowers_calculation_types() {
        let def = parse_def(quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
                price: i64;
                quantity: i64;
            }
            calculations {
                title_len: Option<i64> = string_length(title);
                total: i64 = price * quantity;
            }
            actions {
                read read { primary; }
            }
        }});
        let out = expand_ide_probe(&def).to_string();
        assert!(
            out.contains("__AshStrLen"),
            "missing string-length trait: {out}"
        );
        assert!(
            out.contains("__ash_calc_fits"),
            "missing declared-type fit: {out}"
        );
        assert!(
            out.contains("__ash_str_len"),
            "missing length probe: {out}"
        );
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

    #[test]
    fn test_probe_emits_filter_expr_like_set() {
        let def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    archived: bool;
                }
                actions {
                    read read {
                        primary;
                        prepare filter(archived == false);
                    }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("archived"), "missing filter field: {out}");
        assert!(out.contains("eq"), "missing eq operator: {out}");
        assert!(out.contains("Filter"), "missing Filter type: {out}");
        assert!(out.contains("TestResource"), "missing resource path: {out}");
    }

    #[test]
    fn test_probe_policy_eq_is_nil_and_relates_to() {
        let def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    author_id: Uuid;
                    assignee_id: Option<Uuid>;
                    status: String;
                }
                relationships {
                    belongs_to author: User [fk: author_id];
                }
                actions {
                    read read { primary; }
                }
                policies {
                    policy always {
                        authorize_if relates_to(author);
                        authorize_if is_nil(assignee_id);
                        authorize_if eq(status, "open");
                    }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("author"), "missing relates_to rel: {out}");
        assert!(
            out.contains("__ash_rel"),
            "relates_to should complete from the relationship namespace: {out}"
        );
        assert!(
            out.contains("assignee_id"),
            "missing is_nil field: {out}"
        );
        assert!(out.contains("status"), "missing eq field: {out}");
        assert!(
            out.contains("__ash_assert_assignable"),
            "missing eq literal type check: {out}"
        );
        assert!(out.contains("open"), "missing eq literal: {out}");
    }

    #[test]
    fn test_probe_custom_and_before_action_paths() {
        let def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                calculations {
                    ranked: Option<String> = custom(my_rank_calculator);
                }
                actions {
                    create open {
                        primary;
                        accept [title];
                        change before_action(normalize_title);
                        change custom(&AuditLogger);
                        change func(audit_logger);
                        validate custom(&MyValidator);
                        validate func(check_title);
                    }
                    read read { primary; }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(
            out.contains("BeforeActionFn"),
            "missing before_action type: {out}"
        );
        assert!(
            out.contains("normalize_title"),
            "missing before_action path: {out}"
        );
        assert!(
            out.contains("CustomChange"),
            "missing CustomChange bound: {out}"
        );
        assert!(out.contains("AuditLogger"), "missing custom change path: {out}");
        assert!(
            out.contains("ChangeContext"),
            "missing change func context: {out}"
        );
        assert!(
            out.contains("CustomValidation"),
            "missing CustomValidation bound: {out}"
        );
        assert!(
            out.contains("MyValidator"),
            "missing custom validation path: {out}"
        );
        assert!(
            out.contains("ValidationContext"),
            "missing validation func context: {out}"
        );
        assert!(
            out.contains("my_rank_calculator"),
            "missing calc custom path: {out}"
        );
        assert!(
            out.contains("FieldMap"),
            "missing calc custom signature: {out}"
        );
    }

    #[test]
    fn test_probe_emits_slot_namespaces_not_builder_calls() {
        let def = parse_def(quote! {
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
                actor {
                    role: String;
                }
                actions {
                    create open {
                        primary;
                        accept [title];
                    }
                    read read { primary; }
                    update assign { accept [title]; }
                }
                policies {
                    policy action(open) | action(assign) {
                        authorize_if actor_eq(role = "admin");
                    }
                    policy action_type(read) {
                        authorize_if always;
                    }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(
            out.contains("__AshAcceptFields"),
            "missing accept namespace: {out}"
        );
        assert!(
            out.contains("__AshRelFields"),
            "missing rel namespace: {out}"
        );
        assert!(
            out.contains("__AshActionNames"),
            "missing action namespace: {out}"
        );
        assert!(
            out.contains("__AshActorFields"),
            "missing actor namespace: {out}"
        );
        assert!(out.contains("__ash_accept"), "missing accept bind: {out}");
        assert!(
            out.contains("__ash_actions"),
            "missing action bind: {out}"
        );
        assert!(
            !out.contains("Context < :: ash_memory :: Memory >"),
            "policy action probes should not instantiate builders: {out}"
        );
    }

    #[test]
    fn test_probe_store_through_and_default_fn() {
        let def = parse_def(quote! {
            TestResource {
                store PrimaryDb;
                attributes {
                    id: Uuid [pk];
                    token: String [default_fn: gen_token];
                }
                relationships {
                    many_to_many tags: Tag [through: PostTag, source_fk: post_id, dest_fk: tag_id];
                }
                actions {
                    read read { primary; }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("StoreTag"), "missing store bound: {out}");
        assert!(out.contains("PrimaryDb"), "missing store type: {out}");
        assert!(out.contains("PostTag"), "missing through type: {out}");
        assert!(out.contains("gen_token"), "missing default_fn: {out}");
    }

    #[test]
    fn test_probe_notifiers_and_extensions() {
        let def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                }
                extensions [
                    &STATE_MACHINE
                ]
                notifiers [
                    &AUDIT_LOG_NOTIFIER
                ]
                actions {
                    read read { primary; }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("Notifier"), "missing Notifier bound: {out}");
        assert!(
            out.contains("AUDIT_LOG_NOTIFIER"),
            "missing notifier path: {out}"
        );
        assert!(
            out.contains("ResourceExtension"),
            "missing ResourceExtension bound: {out}"
        );
        assert!(
            out.contains("STATE_MACHINE"),
            "missing extension path: {out}"
        );
    }

    #[test]
    fn test_probe_empty_accept_emits_slot_cursor() {
        let def = parse_def(quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    subject: String;
                }
                actions {
                    create open {
                        accept [];
                    }
                }
            }
        });
        let out = expand_ide_probe(&def).to_string();
        assert!(out.contains("__ash_slot"), "missing empty accept cursor: {out}");
        assert!(
            out.contains("rust_analyzer"),
            "empty accept cursor must be rust-analyzer only: {out}"
        );
        assert!(
            out.contains("__ash_accept"),
            "empty accept should complete from the attribute namespace: {out}"
        );
    }
}
