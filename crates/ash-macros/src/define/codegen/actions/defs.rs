use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::Result;
use syn::spanned::Spanned;

use super::super::policies::lit_to_const_value;
use crate::ast_helpers::argument_attr_type;
use crate::define::ast::{ChangeSpec, PreparationSpec, ResourceDefinition, ValidationSpec};

pub(crate) fn filter_expr_to_tokens(expr: &syn::Expr, resource: &syn::Ident) -> TokenStream {
    match expr {
        syn::Expr::Binary(b) => {
            let left = filter_expr_to_tokens(&b.left, resource);
            let right = filter_expr_to_tokens(&b.right, resource);
            match &b.op {
                syn::BinOp::Eq(_) => quote_spanned! { b.span() => (#left).eq(#right) },
                syn::BinOp::Ne(_) => quote_spanned! { b.span() => (#left).ne(#right) },
                syn::BinOp::Lt(_) => quote_spanned! { b.span() => (#left).lt(#right) },
                syn::BinOp::Le(_) => quote_spanned! { b.span() => (#left).lte(#right) },
                syn::BinOp::Gt(_) => quote_spanned! { b.span() => (#left).gt(#right) },
                syn::BinOp::Ge(_) => quote_spanned! { b.span() => (#left).gte(#right) },
                syn::BinOp::And(_) | syn::BinOp::BitAnd(_) => {
                    quote_spanned! { b.span() => (#left) & (#right) }
                }
                syn::BinOp::Or(_) | syn::BinOp::BitOr(_) => {
                    quote_spanned! { b.span() => (#left) | (#right) }
                }
                _ => quote_spanned! { expr.span() => #expr },
            }
        }
        syn::Expr::MethodCall(m) => {
            let receiver = filter_expr_to_tokens(&m.receiver, resource);
            let method = &m.method;
            let args = &m.args;
            quote_spanned! { m.span() => (#receiver).#method(#args) }
        }
        syn::Expr::Path(p) if p.path.get_ident().is_some() => {
            let id = p.path.get_ident().unwrap();
            if id == "Self" {
                quote_spanned! { id.span() => #resource }
            } else {
                quote_spanned! { id.span() => #resource::#id }
            }
        }
        syn::Expr::Path(p) if p.path.segments.len() == 2 && p.path.segments[0].ident == "Self" => {
            let id = &p.path.segments[1].ident;
            quote_spanned! { id.span() => #resource::#id }
        }
        syn::Expr::Paren(p) => {
            let inner = filter_expr_to_tokens(&p.expr, resource);
            quote_spanned! { p.span() => (#inner) }
        }
        syn::Expr::Unary(u) if matches!(u.op, syn::UnOp::Not(_)) => {
            let inner = filter_expr_to_tokens(&u.expr, resource);
            quote_spanned! { u.span() => !(#inner) }
        }
        _ => quote_spanned! { expr.span() => #expr },
    }
}

fn set_change_tokens(field: &syn::Ident, value: &syn::Expr, new: bool) -> Result<TokenStream> {
    let field_str = field.to_string();
    if let syn::Expr::Lit(syn::ExprLit { lit, .. }) = value {
        let const_val = lit_to_const_value(lit)?;
        if new {
            Ok(
                quote! { ::ash_core::Change::SetNewAttribute { field: #field_str, value: #const_val } },
            )
        } else {
            Ok(quote! { ::ash_core::Change::SetAttribute { field: #field_str, value: #const_val } })
        }
    } else if new {
        Ok(quote_spanned! { value.span() =>
            ::ash_core::Change::SetNewAttributeFn {
                field: #field_str,
                value: || ::ash_core::Value::from(#value),
            }
        })
    } else {
        Ok(quote_spanned! { value.span() =>
            ::ash_core::Change::SetAttributeFn {
                field: #field_str,
                value: || ::ash_core::Value::from(#value),
            }
        })
    }
}

pub fn expand_action_defs(def: &ResourceDefinition) -> Result<(Vec<TokenStream>, bool)> {
    let mut action_defs = Vec::new();
    let mut has_primary_read = false;
    let actions = &def.actions;
    let resource = &def.resource;

    for act in actions {
        let name_str = act.name.to_string();
        let method = act.kind.method_ident();
        let mut builder_chain = quote! { ::ash_core::ActionDef::#method(#name_str) };

        if !act.accept.is_empty() {
            let accept_strs: Vec<String> = act.accept.iter().map(|a| a.name.to_string()).collect();
            builder_chain = quote! { #builder_chain.accept(&[#(#accept_strs),*]) };
        }

        if !act.arguments.is_empty() {
            let arg_tokens: Vec<_> = act
                .arguments
                .iter()
                .map(|arg| {
                    let name_str = arg.name.to_string();
                    let allow_nil = arg.allow_nil;
                    let ty_tokens = argument_attr_type(&arg.ty);
                    let default = match &arg.default {
                        Some(expr) => {
                            let ty = crate::ast_helpers::option_inner(&arg.ty).unwrap_or(&arg.ty);
                            quote! { ::std::option::Option::Some(|| { let value: #ty = #expr; ::ash_core::Value::from(value) }) }
                        }
                        None => quote! { ::std::option::Option::None },
                    };
                    quote! {
                        ::ash_core::ArgumentDef {
                            name: #name_str,
                            ty: #ty_tokens,
                            allow_nil: #allow_nil,
                            default: #default,
                        }
                    }
                })
                .collect();
            builder_chain = quote! { #builder_chain.arguments(&[#(#arg_tokens),*]) };
        }

        if !act.changes.is_empty() {
            let change_tokens = act
                .changes
                .iter()
                .map(|ch| match ch {
                    ChangeSpec::Set { field, value } => set_change_tokens(field, value, false),
                    ChangeSpec::SetNew { field, value } => set_change_tokens(field, value, true),
                    ChangeSpec::RelateActor { field } => {
                        let field_str = field.to_string();
                        Ok(quote! { ::ash_core::Change::RelateActor { field: #field_str } })
                    }
                    ChangeSpec::AtomicUpdate { field, expr } => {
                        let field_str = field.to_string();
                        let expr_tokens = crate::define::codegen::calculations::calc_expr_to_tokens(expr);
                        Ok(quote! { ::ash_core::Change::AtomicUpdate { field: #field_str, expr: &#expr_tokens } })
                    }
                    ChangeSpec::SetFromArg { field, argument } => {
                        let field_str = field.to_string();
                        let arg_str = argument.to_string();
                        Ok(quote! { ::ash_core::Change::SetFromArgument { field: #field_str, argument: #arg_str } })
                    }
                    ChangeSpec::ManageRelationship { relationship, rel_type } => {
                        let rel_str = relationship.to_string();
                        let type_str = rel_type.to_string().to_lowercase();
                        let type_tokens = match type_str.as_str() {
                            "create" => quote! { ::ash_core::ManagedRelType::Create },
                            "append" => quote! { ::ash_core::ManagedRelType::Append },
                            _ => quote! { ::ash_core::ManagedRelType::DirectControl },
                        };
                        Ok(quote! {
                            ::ash_core::Change::ManageRelationship {
                                relationship: #rel_str,
                                rel_type: #type_tokens,
                            }
                        })
                    }
                    ChangeSpec::BeforeAction(expr) => {
                        Ok(quote! { ::ash_core::Change::BeforeAction(#expr) })
                    }
                    ChangeSpec::AfterAction(expr) => {
                        Ok(quote! { ::ash_core::Change::AfterAction(#expr) })
                    }
                    ChangeSpec::AfterTransaction(expr) => {
                        Ok(quote! { ::ash_core::Change::AfterTransaction(#expr) })
                    }
                    ChangeSpec::Custom(expr) => {
                        Ok(quote! { ::ash_core::Change::Custom(#expr) })
                    }
                    ChangeSpec::Func(expr) => {
                        Ok(quote! { ::ash_core::Change::Func(#expr) })
                    }
                })
                .collect::<Result<Vec<_>>>()?;
            builder_chain = quote! {
                {
                    const CHANGES: &'static [::ash_core::Change] = &[#(#change_tokens),*];
                    #builder_chain.changes(CHANGES)
                }
            };
        }

        if !act.validations.is_empty() {
            let validation_tokens = act.validations.iter().map(|v| match v {
                ValidationSpec::Present { field } => {
                    let field_str = field.to_string();
                    quote! { ::ash_core::Validation::Present { field: #field_str } }
                }
                ValidationSpec::StringLength { field, min, max } => {
                    let field_str = field.to_string();
                    let min_tokens = match min {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    let max_tokens = match max {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    quote! {
                        ::ash_core::Validation::StringLength {
                            field: #field_str,
                            min: #min_tokens,
                            max: #max_tokens,
                        }
                    }
                }
                ValidationSpec::OneOf { field, allowed } => {
                    let field_str = field.to_string();
                    quote! {
                        ::ash_core::Validation::OneOf {
                            field: #field_str,
                            allowed: &[#(#allowed),*],
                        }
                    }
                }
                ValidationSpec::Numericality { field, min, max } => {
                    let field_str = field.to_string();
                    let min_tokens = match min {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    let max_tokens = match max {
                        Some(n) => quote! { ::std::option::Option::Some(#n) },
                        None => quote! { ::std::option::Option::None },
                    };
                    quote! {
                        ::ash_core::Validation::Numericality {
                            field: #field_str,
                            min: #min_tokens,
                            max: #max_tokens,
                        }
                    }
                }
                ValidationSpec::Custom(expr) => {
                    quote! { ::ash_core::Validation::Custom(#expr) }
                }
                ValidationSpec::Func(expr) => {
                    quote! { ::ash_core::Validation::Func(#expr) }
                }
            });
            builder_chain = quote! {
                {
                    const VALIDATIONS: &'static [::ash_core::Validation] = &[#(#validation_tokens),*];
                    #builder_chain.validations(VALIDATIONS)
                }
            };
        }

        if !act.preparations.is_empty() {
            let mut prep_helpers = Vec::new();
            let mut prep_tokens = Vec::new();
            for (idx, prep) in act.preparations.iter().enumerate() {
                match prep {
                    PreparationSpec::Filter { expr } => {
                        let prep_fn_name = format_ident!("__prep_{}_{}_filter", act.name, idx);
                        let filter_tokens = filter_expr_to_tokens(expr, resource);
                        prep_helpers.push(quote! {
                            fn #prep_fn_name() -> ::ash_core::Filter {
                                #filter_tokens
                            }
                        });
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Filter(#prep_fn_name)
                        });
                    }
                    PreparationSpec::Sort { field, descending } => {
                        let f_str = field.to_string();
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Sort { field: #f_str, descending: #descending }
                        });
                    }
                    PreparationSpec::Limit(limit) => {
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Limit(#limit)
                        });
                    }
                    PreparationSpec::Offset(offset) => {
                        prep_tokens.push(quote! {
                            ::ash_core::PreparationDef::Offset(#offset)
                        });
                    }
                }
            }
            builder_chain = quote! {
                {
                    #(#prep_helpers)*
                    const PREPARATIONS: &'static [::ash_core::PreparationDef] = &[#(#prep_tokens),*];
                    #builder_chain.preparations(PREPARATIONS)
                }
            };
        }

        if act.primary {
            has_primary_read = true;
            builder_chain = quote! { #builder_chain.primary() };
        }

        if act.persist_manual {
            builder_chain = quote! { #builder_chain.manual() };
        }

        if let Some(read) = &act.atomic_upgrade_with {
            let read = read.to_string();
            builder_chain = quote! { #builder_chain.atomic_upgrade_with(#read) };
        }
        if let Some(spec) = &act.pagination {
            let (keyset, offset) = (spec.keyset, spec.offset);
            let countable = match spec.countable.as_ref().map(|ident| ident.to_string()) {
                Some(value) if value == "true" => quote! { ::ash_core::Countable::Yes },
                Some(value) if value == "by_default" => quote! { ::ash_core::Countable::ByDefault },
                _ => quote! { ::ash_core::Countable::No },
            };
            let default_limit = match &spec.default_limit {
                Some(limit) => quote! { ::std::option::Option::Some(#limit) },
                None => quote! { ::std::option::Option::None },
            };
            let max_page_size = match &spec.max_page_size {
                Some(Some(max)) => quote! { ::std::option::Option::Some(#max) },
                Some(None) => quote! { ::std::option::Option::None },
                None => quote! { ::std::option::Option::Some(250) },
            };
            let required = spec.required.unwrap_or(true);
            builder_chain = quote! {
                #builder_chain.pagination(::ash_core::Pagination {
                    keyset: #keyset,
                    offset: #offset,
                    countable: #countable,
                    default_limit: #default_limit,
                    max_page_size: #max_page_size,
                    required: #required,
                })
            };
        }
        if !act.require_atomic {
            builder_chain = quote! { #builder_chain.require_atomic(false) };
        }
        if act.soft {
            builder_chain = quote! { #builder_chain.soft() };
        }

        if !act.cascade_destroy.is_empty() {
            let rels: Vec<String> = act.cascade_destroy.iter().map(|r| r.to_string()).collect();
            builder_chain = quote! { #builder_chain.cascade_destroy(&[#(#rels),*]) };
        }

        action_defs.push(builder_chain);
    }

    Ok((action_defs, has_primary_read))
}
