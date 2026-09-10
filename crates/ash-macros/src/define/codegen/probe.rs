use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};

use crate::define::ast::{ChangeSpec, PreparationSpec, ResourceDefinition, ValidationSpec};

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
        for arg in &action.arguments {
            let arg_name = &arg.name;
            let arg_ty = &arg.ty;
            field_probes.push(quote_spanned! { arg_name.span() =>
                let #arg_name: &#arg_ty = loop {};
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
                ChangeSpec::Set { field, .. }
                | ChangeSpec::SetNew { field, .. }
                | ChangeSpec::RelateActor { field } => {
                    field_probes.push(quote_spanned! { field.span() =>
                        let _ = &__ash_record.#field;
                    });
                }
                ChangeSpec::SetFromArg { field, argument } => {
                    field_probes.push(quote_spanned! { field.span() =>
                        let _ = &__ash_record.#field;
                    });
                    field_probes.push(quote_spanned! { argument.span() =>
                        let _ = &#argument;
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

    quote! {
        #[doc(hidden)]
        const _: () = {
            #[allow(
                dead_code,
                unused_variables,
                non_snake_case,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_ide_typecheck(__ash_record: &#resource) {
                if false {
                    #(#action_probes)*
                }
            }
        };
    }
}
