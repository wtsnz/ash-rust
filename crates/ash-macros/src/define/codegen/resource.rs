use super::calculations::calc_expr_to_tokens;
use super::policies::lit_to_const_value;
use crate::ast_helpers::{
    is_bool, is_integer, is_string, is_uuid, option_inner, screaming_snake, snake_case,
};
use crate::define::ast::{AggregateFilterSpec, AggregateKindSpec, RelType, ResourceDefinition};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Error, Result, Type};

fn try_from_i64(ty: &Type, n: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    quote! {
        <#ty as ::std::convert::TryFrom<i64>>::try_from(#n).map_err(|_| {
            ::ash_core::Error::Invalid(::std::format!(
                "integer does not fit in {}",
                stringify!(#ty)
            ))
        })?
    }
}

fn composite_keys(
    rel: &crate::define::ast::RelationshipSpec,
    name: &str,
    resource: &str,
) -> Result<TokenStream> {
    if rel.fk_columns.len() <= 1 && rel.reference_columns.is_empty() {
        return Ok(quote! {});
    }
    if matches!(rel.kind, RelType::ManyToMany) {
        return Err(Error::new_spanned(
            &rel.ident,
            "many_to_many does not take a composite foreign key",
        ));
    }
    let source_default = match rel.kind {
        RelType::BelongsTo => rel
            .fk
            .as_ref()
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("{name}_id")),
        _ => "id".to_string(),
    };
    let dest_default = match rel.kind {
        RelType::BelongsTo => "id".to_string(),
        _ => rel
            .fk
            .as_ref()
            .map(|id| id.to_string())
            .unwrap_or_else(|| format!("{}_id", snake_case(resource))),
    };
    let (source, destination) = match rel.kind {
        RelType::BelongsTo => {
            let source = if rel.fk_columns.is_empty() {
                vec![source_default]
            } else {
                rel.fk_columns.iter().map(|id| id.to_string()).collect()
            };
            let destination = if rel.reference_columns.is_empty() {
                vec![dest_default]
            } else {
                rel.reference_columns
                    .iter()
                    .map(|id| id.to_string())
                    .collect()
            };
            (source, destination)
        }
        _ => {
            let destination = if rel.fk_columns.is_empty() {
                vec![dest_default]
            } else {
                rel.fk_columns.iter().map(|id| id.to_string()).collect()
            };
            let source = if rel.reference_columns.is_empty() {
                vec![source_default]
            } else {
                rel.reference_columns
                    .iter()
                    .map(|id| id.to_string())
                    .collect()
            };
            (source, destination)
        }
    };
    if source.len() != destination.len() {
        return Err(Error::new_spanned(
            &rel.ident,
            format!(
                "relationship `{name}` has {} key columns and {} referenced columns",
                source.len(),
                destination.len()
            ),
        ));
    }
    Ok(quote! { .with_keys(&[#(#source),*], &[#(#destination),*]) })
}

pub fn expand_resource_struct(def: &ResourceDefinition) -> Result<TokenStream> {
    if def.attributes.is_empty() {
        return Ok(quote! {});
    }

    let resource = &def.resource;
    let resource_str = resource.to_string();

    let table_str = if def.embedded {
        def.table.clone().unwrap_or_default()
    } else {
        def.table
            .clone()
            .unwrap_or_else(|| snake_case(&resource_str))
    };

    let pk_attr = def.attributes.iter().find(|a| a.pk);
    let (pk_fn_body, has_pk) = match pk_attr {
        Some(pk) => {
            let pk_id = &pk.ident;
            (quote! { self.#pk_id }, true)
        }
        None => {
            if def.embedded {
                (quote! { ::uuid::Uuid::nil() }, false)
            } else {
                return Err(Error::new_spanned(
                    resource,
                    "Resource requires an attribute marked [pk]",
                ));
            }
        }
    };
    let _ = has_pk;

    // 1. Struct fields
    let mut struct_fields = Vec::new();
    for a in &def.attributes {
        let o_attrs = &a.outer_attrs;
        let id = &a.ident;
        let ty = &a.ty;
        struct_fields.push(quote! { #(#o_attrs)* pub #id: #ty });
    }
    for c in &def.calculations {
        let o_attrs = &c.outer_attrs;
        let id = &c.ident;
        let inner = option_inner(&c.ty).unwrap_or(&c.ty);
        struct_fields.push(quote! { #(#o_attrs)* pub #id: ::std::option::Option<#inner> });
    }
    for agg in &def.aggregates {
        let o_attrs = &agg.outer_attrs;
        let id = &agg.ident;
        let ty = &agg.ty;
        struct_fields.push(quote! { #(#o_attrs)* pub #id: #ty });
    }
    for r in &def.relationships {
        let o_attrs = &r.outer_attrs;
        let id = &r.ident;
        let ty = &r.struct_field_ty;
        struct_fields.push(quote! { #(#o_attrs)* pub #id: #ty });
    }

    // 2. AttributeDefs
    let mut attr_defs = Vec::new();
    let mut helper_default_fns = Vec::new();
    for a in &def.attributes {
        let name_str = a.ident.to_string();
        let ty = &a.ty;
        if a.pk {
            attr_defs.push(quote! { ::ash_core::AttributeDef::uuid_pk(#name_str) });
        } else if a.version || def.optimistic_lock.as_ref() == Some(&a.ident) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::version(#name_str) });
        } else if a.uses_ash_type_storage() {
            let inner_ty = option_inner(ty).unwrap_or(ty);
            if let Some(default_expr) = &a.default {
                let fn_name = format_ident!("__default_{}", name_str);
                helper_default_fns.push(quote! {
                    fn #fn_name() -> ::ash_core::Value {
                        ::ash_core::Value::from(#default_expr)
                    }
                });
                attr_defs.push(quote! {
                    ::ash_core::AttributeDef::with_default(
                        #name_str,
                        <#inner_ty as ::ash_core::AshType>::ATTR_TYPE,
                        #fn_name
                    )
                });
            } else if option_inner(ty).is_some() {
                attr_defs.push(quote! {
                    ::ash_core::AttributeDef::optional(
                        #name_str,
                        <#inner_ty as ::ash_core::AshType>::ATTR_TYPE
                    )
                });
            } else {
                attr_defs.push(quote! {
                    ::ash_core::AttributeDef::required(
                        #name_str,
                        <#inner_ty as ::ash_core::AshType>::ATTR_TYPE
                    )
                });
            }
        } else if let Some(default_expr) = &a.default {
            let fn_name = format_ident!("__default_{}", name_str);
            let attr_ty = if is_string(ty) {
                quote! { ::ash_core::AttrType::String }
            } else if is_integer(ty) {
                quote! { ::ash_core::AttrType::Integer }
            } else if is_bool(ty) {
                quote! { ::ash_core::AttrType::Boolean }
            } else {
                quote! { ::ash_core::AttrType::Map }
            };
            helper_default_fns.push(quote! {
                fn #fn_name() -> ::ash_core::Value {
                    ::ash_core::Value::from(#default_expr)
                }
            });
            attr_defs.push(
                quote! { ::ash_core::AttributeDef::with_default(#name_str, #attr_ty, #fn_name) },
            );
        } else if let Some(default_fn_path) = &a.default_fn {
            let fn_name = format_ident!("__default_{}", name_str);
            let attr_ty = if is_string(ty) {
                quote! { ::ash_core::AttrType::String }
            } else if is_integer(ty) {
                quote! { ::ash_core::AttrType::Integer }
            } else if is_bool(ty) {
                quote! { ::ash_core::AttrType::Boolean }
            } else {
                quote! { ::ash_core::AttrType::Map }
            };
            helper_default_fns.push(quote! {
                fn #fn_name() -> ::ash_core::Value {
                    ::ash_core::Value::from((#default_fn_path)())
                }
            });
            attr_defs.push(
                quote! { ::ash_core::AttributeDef::with_default(#name_str, #attr_ty, #fn_name) },
            );
        } else if a.generated {
            attr_defs.push(quote! { ::ash_core::AttributeDef::generated(#name_str, ::ash_core::AttrType::String) });
        } else if let Some(atoms) = &a.atom {
            attr_defs.push(quote! {
                ::ash_core::AttributeDef::required(
                    #name_str,
                    ::ash_core::AttrType::Atom { one_of: &[#(#atoms),*] }
                )
            });
        } else if is_uuid(ty) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::required(#name_str, ::ash_core::AttrType::Uuid) });
        } else if option_inner(ty).is_some_and(is_uuid) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::optional(#name_str, ::ash_core::AttrType::Uuid) });
        } else if is_string(ty) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::required(#name_str, ::ash_core::AttrType::String) });
        } else if option_inner(ty).is_some_and(is_string) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::optional(#name_str, ::ash_core::AttrType::String) });
        } else if is_integer(ty) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::required(#name_str, ::ash_core::AttrType::Integer) });
        } else if option_inner(ty).is_some_and(is_integer) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::optional(#name_str, ::ash_core::AttrType::Integer) });
        } else if is_bool(ty) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::required(#name_str, ::ash_core::AttrType::Boolean) });
        } else if option_inner(ty).is_some_and(is_bool) {
            attr_defs.push(quote! { ::ash_core::AttributeDef::optional(#name_str, ::ash_core::AttrType::Boolean) });
        } else if option_inner(ty).is_some() {
            attr_defs.push(
                quote! { ::ash_core::AttributeDef::optional(#name_str, ::ash_core::AttrType::Map) },
            );
        } else {
            attr_defs.push(
                quote! { ::ash_core::AttributeDef::required(#name_str, ::ash_core::AttrType::Map) },
            );
        }
    }

    // 3. RelationshipDefs
    let mut rel_defs = Vec::new();
    for r in &def.relationships {
        let name_str = r.ident.to_string();
        let dest = &r.dest;
        let on_delete_tok = match r.on_delete {
            crate::define::ast::OnDeleteSpec::Nothing => quote! { ::ash_core::OnDelete::Nothing },
            crate::define::ast::OnDeleteSpec::Cascade => quote! { ::ash_core::OnDelete::Cascade },
            crate::define::ast::OnDeleteSpec::Nilify => quote! { ::ash_core::OnDelete::Nilify },
            crate::define::ast::OnDeleteSpec::Restrict => quote! { ::ash_core::OnDelete::Restrict },
        };
        let on_update_tok = match r.on_update {
            crate::define::ast::OnDeleteSpec::Nothing => quote! { ::ash_core::OnUpdate::Nothing },
            crate::define::ast::OnDeleteSpec::Cascade => quote! { ::ash_core::OnUpdate::Cascade },
            crate::define::ast::OnDeleteSpec::Nilify => quote! { ::ash_core::OnUpdate::Nilify },
            crate::define::ast::OnDeleteSpec::Restrict => quote! { ::ash_core::OnUpdate::Restrict },
        };
        let keys = composite_keys(r, &name_str, &resource_str)?;
        match r.kind {
            RelType::BelongsTo => {
                let fk_str =
                    r.fk.as_ref()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| format!("{name_str}_id"));
                rel_defs.push(quote! {
                    ::ash_core::RelationshipDef::belongs_to(
                        #name_str,
                        || &<#dest as ::ash_core::Resource>::DEF,
                        #fk_str,
                    ).with_on_delete(#on_delete_tok).with_on_update(#on_update_tok)#keys
                });
            }
            RelType::HasMany => {
                let fk_str =
                    r.fk.as_ref()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| format!("{}_id", snake_case(&resource_str)));
                rel_defs.push(quote! {
                    ::ash_core::RelationshipDef::has_many(
                        #name_str,
                        || &<#dest as ::ash_core::Resource>::DEF,
                        #fk_str,
                    ).with_on_delete(#on_delete_tok).with_on_update(#on_update_tok)#keys
                });
            }
            RelType::HasOne => {
                let fk_str =
                    r.fk.as_ref()
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| format!("{}_id", snake_case(&resource_str)));
                rel_defs.push(quote! {
                    ::ash_core::RelationshipDef::has_one(
                        #name_str,
                        || &<#dest as ::ash_core::Resource>::DEF,
                        #fk_str,
                    ).with_on_delete(#on_delete_tok).with_on_update(#on_update_tok)#keys
                });
            }
            RelType::ManyToMany => {
                let through_ident = r.through.as_ref().ok_or_else(|| {
                    syn::Error::new_spanned(
                        &r.ident,
                        "many_to_many requires `through: JoinResource`",
                    )
                })?;
                let source_on_join = r
                    .source_attribute_on_join_resource
                    .clone()
                    .unwrap_or_else(|| format!("{}_id", snake_case(&resource_str)));
                let dest_on_join = r
                    .destination_attribute_on_join_resource
                    .clone()
                    .unwrap_or_else(|| format!("{}_id", snake_case(&dest.to_string())));
                rel_defs.push(quote! {
                    ::ash_core::RelationshipDef::many_to_many(
                        #name_str,
                        || &<#dest as ::ash_core::Resource>::DEF,
                        || &<#through_ident as ::ash_core::Resource>::DEF,
                        #source_on_join,
                        #dest_on_join,
                    ).with_on_delete(#on_delete_tok).with_on_update(#on_update_tok)
                });
            }
        }
    }

    // 4. CalculationDefs
    let mut calc_defs = Vec::new();
    for c in &def.calculations {
        let name_str = c.ident.to_string();
        let inner = option_inner(&c.ty).unwrap_or(&c.ty);
        let ty_tokens = if is_string(inner) {
            quote! { ::ash_core::AttrType::String }
        } else if is_integer(inner) {
            quote! { ::ash_core::AttrType::Integer }
        } else if is_bool(inner) {
            quote! { ::ash_core::AttrType::Boolean }
        } else if is_uuid(inner) {
            quote! { ::ash_core::AttrType::Uuid }
        } else {
            quote! { ::ash_core::AttrType::String }
        };
        let expr_tokens = calc_expr_to_tokens(&c.expr);
        if c.arguments.is_empty() {
            calc_defs.push(quote! {
                ::ash_core::CalculationDef::new(
                    #name_str,
                    #ty_tokens,
                    #expr_tokens,
                )
            });
        } else {
            let arg_defs: Vec<_> = c
                .arguments
                .iter()
                .map(|arg| {
                    let arg_name = arg.name.to_string();
                    let arg_ty = &arg.ty;
                    let arg_inner = option_inner(arg_ty).unwrap_or(arg_ty);
                    let arg_allow_nil = option_inner(arg_ty).is_some();
                    let arg_type_tok = if is_string(arg_inner) {
                        quote! { ::ash_core::AttrType::String }
                    } else if is_integer(arg_inner) {
                        quote! { ::ash_core::AttrType::Integer }
                    } else if is_bool(arg_inner) {
                        quote! { ::ash_core::AttrType::Boolean }
                    } else if is_uuid(arg_inner) {
                        quote! { ::ash_core::AttrType::Uuid }
                    } else {
                        quote! { ::ash_core::AttrType::String }
                    };
                    quote! {
                        ::ash_core::ArgumentDef {
                            name: #arg_name,
                            ty: #arg_type_tok,
                            allow_nil: #arg_allow_nil,
                        }
                    }
                })
                .collect();
            calc_defs.push(quote! {
                ::ash_core::CalculationDef::with_arguments(
                    #name_str,
                    #ty_tokens,
                    #expr_tokens,
                    &[#(#arg_defs),*],
                )
            });
        }
    }

    // 4b. AggregateDefs
    let mut agg_defs = Vec::new();
    for agg in &def.aggregates {
        let name_str = agg.ident.to_string();
        let rel_str = agg.relationship.to_string();
        let ty_tokens = match &agg.kind {
            AggregateKindSpec::Count | AggregateKindSpec::Sum { .. } => {
                quote! { ::ash_core::AttrType::Integer }
            }
            AggregateKindSpec::Exists => {
                quote! { ::ash_core::AttrType::Boolean }
            }
            AggregateKindSpec::First { .. } => {
                let inner = option_inner(&agg.ty).unwrap_or(&agg.ty);
                if is_uuid(inner) {
                    quote! { ::ash_core::AttrType::Uuid }
                } else if is_string(inner) {
                    quote! { ::ash_core::AttrType::String }
                } else if is_integer(inner) {
                    quote! { ::ash_core::AttrType::Integer }
                } else if is_bool(inner) {
                    quote! { ::ash_core::AttrType::Boolean }
                } else {
                    quote! { ::ash_core::AttrType::String }
                }
            }
        };

        let kind_tokens = match &agg.kind {
            AggregateKindSpec::Count => quote! { ::ash_core::AggregateKind::Count },
            AggregateKindSpec::Exists => quote! { ::ash_core::AggregateKind::Exists },
            AggregateKindSpec::First { field } => {
                let f_str = field.to_string();
                quote! { ::ash_core::AggregateKind::First { field: #f_str } }
            }
            AggregateKindSpec::Sum { field } => {
                let f_str = field.to_string();
                quote! { ::ash_core::AggregateKind::Sum { field: #f_str } }
            }
        };

        let filter_tokens = match &agg.filter {
            None => quote! { ::std::option::Option::None },
            Some(AggregateFilterSpec::Eq { field, value }) => {
                let f_str = field.to_string();
                let const_val = lit_to_const_value(value)?;
                quote! {
                    ::std::option::Option::Some(::ash_core::AggregateFilter::Eq(#f_str, #const_val))
                }
            }
            Some(AggregateFilterSpec::Ne { field, value }) => {
                let f_str = field.to_string();
                let const_val = lit_to_const_value(value)?;
                quote! {
                    ::std::option::Option::Some(::ash_core::AggregateFilter::Ne(#f_str, #const_val))
                }
            }
        };

        agg_defs.push(quote! {
            ::ash_core::AggregateDef {
                name: #name_str,
                relationship: #rel_str,
                kind: #kind_tokens,
                filter: #filter_tokens,
                ty: #ty_tokens,
            }
        });
    }

    // 5. to_fields inserts
    let mut to_inserts = Vec::new();
    for a in &def.attributes {
        let id = &a.ident;
        let name_str = id.to_string();
        let ty = &a.ty;
        if a.uses_ash_type_storage() {
            if option_inner(ty).is_some() {
                to_inserts.push(quote! {
                    if let ::std::option::Option::Some(val) = &self.#id {
                        map.insert(
                            ::std::string::String::from(#name_str),
                            ::ash_core::AshType::to_value(val),
                        );
                    } else {
                        map.insert(
                            ::std::string::String::from(#name_str),
                            ::ash_core::Value::Null,
                        );
                    }
                });
            } else {
                to_inserts.push(quote! {
                    map.insert(
                        ::std::string::String::from(#name_str),
                        ::ash_core::AshType::to_value(&self.#id),
                    );
                });
            }
        } else if a.atom.is_some() {
            to_inserts.push(quote! {
                map.insert(
                    ::std::string::String::from(#name_str),
                    ::ash_core::Value::from(self.#id.as_str()),
                );
            });
        } else if is_string(ty) {
            to_inserts.push(quote! {
                map.insert(
                    ::std::string::String::from(#name_str),
                    ::ash_core::Value::from(self.#id.clone()),
                );
            });
        } else if option_inner(ty).is_some() {
            to_inserts.push(quote! {
                if let ::std::option::Option::Some(val) = &self.#id {
                    map.insert(
                        ::std::string::String::from(#name_str),
                        ::ash_core::Value::from(val.clone()),
                    );
                } else {
                    map.insert(
                        ::std::string::String::from(#name_str),
                        ::ash_core::Value::Null,
                    );
                }
            });
        } else {
            to_inserts.push(quote! {
                map.insert(
                    ::std::string::String::from(#name_str),
                    ::ash_core::Value::from(self.#id.clone()),
                );
            });
        }
    }
    for agg in &def.aggregates {
        let id = &agg.ident;
        let name_str = id.to_string();
        to_inserts.push(quote! {
            if let ::std::option::Option::Some(val) = &self.#id {
                map.insert(
                    ::std::string::String::from(#name_str),
                    ::ash_core::Value::from(val.clone()),
                );
            }
        });
    }

    // 6. from_fields inits
    let mut from_inits = Vec::new();
    for a in &def.attributes {
        let id = &a.ident;
        let name_str = id.to_string();
        let ty = &a.ty;

        if a.uses_ash_type_storage() {
            let inner_ty = option_inner(ty).unwrap_or(ty);
            if option_inner(ty).is_some() {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            ::std::option::Option::Some(<#inner_ty as ::ash_core::AshType>::from_value(val)?)
                        }
                        _ => ::std::option::Option::None,
                    }
                });
            } else if let Some(default_expr) = &a.default {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            <#inner_ty as ::ash_core::AshType>::from_value(val)?
                        }
                        _ => <#inner_ty as ::ash_core::AshType>::from_value(
                            &::ash_core::Value::from(#default_expr),
                        )?,
                    }
                });
            } else if let Some(default_fn) = &a.default_fn {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            <#inner_ty as ::ash_core::AshType>::from_value(val)?
                        }
                        _ => <#inner_ty as ::ash_core::AshType>::from_value(&(#default_fn)())?,
                    }
                });
            } else {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(val) if !val.is_null() => {
                            <#inner_ty as ::ash_core::AshType>::from_value(val)?
                        }
                        _ => return Err(::ash_core::Error::Missing { field: #name_str.into() }),
                    }
                });
            }
            continue;
        }

        if let Some(ts) = &def.timestamps
            && (id == &ts.created_at || id == &ts.updated_at)
        {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::String(s)) => s.clone(),
                    _ => ::ash_core::utc_now_iso8601(),
                }
            });
            continue;
        }

        if let Some(default_expr) = &a.default {
            if is_string(ty) {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(::ash_core::Value::String(s)) => s.clone(),
                        _ => ::std::convert::Into::into(#default_expr),
                    }
                });
                continue;
            } else if is_integer(ty) {
                let conv = try_from_i64(ty, quote!(*n));
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(::ash_core::Value::Int(n)) => #conv,
                        _ => #default_expr,
                    }
                });
                continue;
            } else if is_bool(ty) {
                from_inits.push(quote! {
                    #id: match fields.get(#name_str) {
                        ::std::option::Option::Some(::ash_core::Value::Bool(b)) => *b,
                        _ => #default_expr,
                    }
                });
                continue;
            }
        }

        if let Some(default_fn) = &a.default_fn
            && is_string(ty)
        {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::String(s)) => s.clone(),
                    _ => ::std::convert::Into::into((#default_fn)()),
                }
            });
            continue;
        }

        if a.pk || is_uuid(ty) {
            from_inits.push(quote! { #id: ::ash_core::required_uuid(fields, #name_str)? });
        } else if option_inner(ty).is_some_and(is_uuid) {
            from_inits.push(quote! { #id: ::ash_core::optional_uuid(fields, #name_str)? });
        } else if is_string(ty) {
            from_inits.push(quote! { #id: ::ash_core::required_string(fields, #name_str)? });
        } else if option_inner(ty).is_some_and(is_string) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::String(s)) => ::std::option::Option::Some(s.clone()),
                    _ => ::std::option::Option::None,
                }
            });
        } else if a.atom.is_some() {
            from_inits.push(
                quote! { #id: #ty::parse(&::ash_core::required_string(fields, #name_str)?)? },
            );
        } else if is_integer(ty) {
            let conv = try_from_i64(ty, quote!(n));
            from_inits.push(quote! {
                #id: {
                    let n = ::ash_core::optional_int(fields, #name_str)?
                        .ok_or_else(|| ::ash_core::Error::Missing { field: #name_str.into() })?;
                    #conv
                }
            });
        } else if option_inner(ty).is_some_and(is_integer) {
            let inner = option_inner(ty).unwrap();
            let conv = try_from_i64(inner, quote!(n));
            from_inits.push(quote! {
                #id: match ::ash_core::optional_int(fields, #name_str)? {
                    ::std::option::Option::Some(n) => ::std::option::Option::Some(#conv),
                    ::std::option::Option::None => ::std::option::Option::None,
                }
            });
        } else if is_bool(ty) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Bool(b)) => *b,
                    _ => return Err(::ash_core::Error::Missing { field: #name_str.into() }),
                }
            });
        } else if option_inner(ty).is_some_and(is_bool) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Bool(b)) => ::std::option::Option::Some(*b),
                    _ => ::std::option::Option::None,
                }
            });
        } else if let Some(inner) = option_inner(ty) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Map(m)) => {
                        ::std::option::Option::Some(<#inner as ::ash_core::Resource>::from_fields(m)?)
                    }
                    _ => ::std::option::Option::None,
                }
            });
        } else {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Map(m)) => {
                        <#ty as ::ash_core::Resource>::from_fields(m)?
                    }
                    _ => return Err(::ash_core::Error::Missing { field: #name_str.into() }),
                }
            });
        }
    }
    for c in &def.calculations {
        let id = &c.ident;
        let name_str = id.to_string();
        let inner = option_inner(&c.ty).unwrap_or(&c.ty);
        if is_integer(inner) {
            let conv = try_from_i64(inner, quote!(n));
            from_inits.push(quote! {
                #id: match ::ash_core::optional_int(fields, #name_str)? {
                    ::std::option::Option::Some(n) => ::std::option::Option::Some(#conv),
                    ::std::option::Option::None => ::std::option::Option::None,
                }
            });
        } else if is_string(inner) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::String(s)) => ::std::option::Option::Some(s.clone()),
                    _ => ::std::option::Option::None,
                }
            });
        } else if is_bool(inner) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Bool(b)) => ::std::option::Option::Some(*b),
                    _ => ::std::option::Option::None,
                }
            });
        } else if is_uuid(inner) {
            from_inits.push(quote! { #id: ::ash_core::optional_uuid(fields, #name_str)? });
        } else {
            from_inits.push(quote! { #id: ::ash_core::optional_int(fields, #name_str)? });
        }
    }
    for agg in &def.aggregates {
        let id = &agg.ident;
        let name_str = id.to_string();
        let inner = option_inner(&agg.ty).unwrap_or(&agg.ty);
        if is_integer(inner) {
            let conv = try_from_i64(inner, quote!(n));
            from_inits.push(quote! {
                #id: match ::ash_core::optional_int(fields, #name_str)? {
                    ::std::option::Option::Some(n) => ::std::option::Option::Some(#conv),
                    ::std::option::Option::None => ::std::option::Option::None,
                }
            });
        } else if is_bool(inner) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::Bool(b)) => ::std::option::Option::Some(*b),
                    _ => ::std::option::Option::None,
                }
            });
        } else if is_string(inner) {
            from_inits.push(quote! {
                #id: match fields.get(#name_str) {
                    ::std::option::Option::Some(::ash_core::Value::String(s)) => ::std::option::Option::Some(s.clone()),
                    _ => ::std::option::Option::None,
                }
            });
        } else if is_uuid(inner) {
            from_inits.push(quote! { #id: ::ash_core::optional_uuid(fields, #name_str)? });
        } else {
            from_inits.push(quote! { #id: ::ash_core::optional_int(fields, #name_str)? });
        }
    }
    for r in &def.relationships {
        let id = &r.ident;
        from_inits.push(quote! { #id: ::ash_core::Rel::NotLoaded });
    }

    // 7. attach arms
    let mut attach_arms = Vec::new();
    for r in &def.relationships {
        let id = &r.ident;
        let name_str = id.to_string();
        let dest = &r.dest;
        match r.kind {
            RelType::BelongsTo | RelType::HasOne => {
                attach_arms.push(quote! {
                    #name_str => {
                        self.#id = ::ash_core::Rel::Loaded(match related.first() {
                            ::std::option::Option::Some(row) => ::std::option::Option::Some(<#dest as ::ash_core::Resource>::from_fields(row)?),
                            ::std::option::Option::None => ::std::option::Option::None,
                        });
                        Ok(())
                    }
                });
            }
            RelType::HasMany | RelType::ManyToMany => {
                attach_arms.push(quote! {
                    #name_str => {
                        self.#id = ::ash_core::Rel::Loaded(
                            related
                                .iter()
                                .map(<#dest as ::ash_core::Resource>::from_fields)
                                .collect::<::ash_core::Result<::std::vec::Vec<_>>>()?,
                        );
                        Ok(())
                    }
                });
            }
        }
    }

    // 8. fields constants
    let mut field_consts = Vec::new();
    let mut associated_field_consts = Vec::new();
    for a in &def.attributes {
        let id = &a.ident;
        let name_str = id.to_string();
        let ty = &a.ty;
        let o_attrs = &a.outer_attrs;
        let has_action_conflict = def.actions.iter().any(|act| act.name == *id);

        if a.uses_ash_type_storage() {
            let inner_ty = option_inner(ty).unwrap_or(ty);
            field_consts.push(quote! {
                #(#o_attrs)*
                pub const #id: ::ash_core::Attr<super::#resource, #inner_ty> =
                    ::ash_core::Attr::new(#name_str);
            });
            if !has_action_conflict {
                associated_field_consts.push(quote! {
                    #(#o_attrs)*
                    pub const #id: ::ash_core::Attr<Self, #inner_ty> =
                        ::ash_core::Attr::new(#name_str);
                });
            }
        } else if a.pk || is_uuid(ty) || option_inner(ty).is_some_and(is_uuid) {
            field_consts.push(quote! {
                #(#o_attrs)*
                pub const #id: ::ash_core::Attr<super::#resource, ::uuid::Uuid> =
                    ::ash_core::Attr::new(#name_str);
            });
            if !has_action_conflict {
                associated_field_consts.push(quote! {
                    #(#o_attrs)*
                    pub const #id: ::ash_core::Attr<Self, ::uuid::Uuid> =
                        ::ash_core::Attr::new(#name_str);
                });
            }
        } else if is_string(ty) || option_inner(ty).is_some_and(is_string) || a.atom.is_some() {
            field_consts.push(quote! {
                #(#o_attrs)*
                pub const #id: ::ash_core::Attr<super::#resource, ::std::string::String> =
                    ::ash_core::Attr::new(#name_str);
            });
            if !has_action_conflict {
                associated_field_consts.push(quote! {
                    #(#o_attrs)*
                    pub const #id: ::ash_core::Attr<Self, ::std::string::String> =
                        ::ash_core::Attr::new(#name_str);
                });
            }
        } else if is_integer(ty) || option_inner(ty).is_some_and(is_integer) {
            let inner = option_inner(ty).unwrap_or(ty);
            field_consts.push(quote! {
                #(#o_attrs)*
                pub const #id: ::ash_core::Attr<super::#resource, #inner> =
                    ::ash_core::Attr::new(#name_str);
            });
            if !has_action_conflict {
                associated_field_consts.push(quote! {
                    #(#o_attrs)*
                    pub const #id: ::ash_core::Attr<Self, #inner> =
                        ::ash_core::Attr::new(#name_str);
                });
            }
        } else if is_bool(ty) || option_inner(ty).is_some_and(is_bool) {
            field_consts.push(quote! {
                #(#o_attrs)*
                pub const #id: ::ash_core::Attr<super::#resource, bool> =
                    ::ash_core::Attr::new(#name_str);
            });
            if !has_action_conflict {
                associated_field_consts.push(quote! {
                    #(#o_attrs)*
                    pub const #id: ::ash_core::Attr<Self, bool> =
                        ::ash_core::Attr::new(#name_str);
                });
            }
        }
    }
    for c in &def.calculations {
        let id = &c.ident;
        let name_str = id.to_string();
        let c_ty = &c.ty;
        let has_action_conflict = def.actions.iter().any(|act| act.name == *id);
        field_consts.push(quote! {
            pub const #id: ::ash_core::Calc<super::#resource, #c_ty> =
                ::ash_core::Calc::new(#name_str);
        });
        if !has_action_conflict {
            associated_field_consts.push(quote! {
                pub const #id: ::ash_core::Calc<Self, #c_ty> =
                    ::ash_core::Calc::new(#name_str);
            });
        }
    }
    for agg in &def.aggregates {
        let id = &agg.ident;
        let name_str = id.to_string();
        let inner = option_inner(&agg.ty).unwrap_or(&agg.ty);
        let has_action_conflict = def.actions.iter().any(|act| act.name == *id);
        field_consts.push(quote! {
            pub const #id: ::ash_core::Aggregate<super::#resource, #inner> =
                ::ash_core::Aggregate::new(#name_str);
        });
        if !has_action_conflict {
            associated_field_consts.push(quote! {
                pub const #id: ::ash_core::Aggregate<Self, #inner> =
                    ::ash_core::Aggregate::new(#name_str);
            });
        }
    }
    for r in &def.relationships {
        let id = &r.ident;
        let name_str = id.to_string();
        let dest = &r.dest;
        let has_action_conflict = def.actions.iter().any(|act| act.name == *id);
        field_consts.push(quote! {
            pub const #id: ::ash_core::Relation<super::#resource, super::#dest> =
                ::ash_core::Relation::new(#name_str);
        });
        if !has_action_conflict {
            associated_field_consts.push(quote! {
                pub const #id: ::ash_core::Relation<Self, #dest> =
                    ::ash_core::Relation::new(#name_str);
            });
        }
    }

    let def_const_name = format_ident!("{}_DEF", screaming_snake(&resource_str));
    let fields_mod_name = format_ident!("{}_fields", snake_case(&resource_str));
    let outer_attrs = &def.outer_attrs;
    let ext_tokens: Vec<_> = def.extensions.iter().map(|ext| quote! { #ext }).collect();
    let notifier_tokens: Vec<_> = def.notifiers.iter().map(|n| quote! { #n }).collect();

    let ident_defs: Vec<_> = def
        .identities
        .iter()
        .map(|ident| {
            let name_str = ident.name.to_string();
            let key_strs: Vec<String> = ident.keys.iter().map(|k| k.to_string()).collect();
            let msg_tokens = match &ident.message {
                Some(msg) => quote! { ::std::option::Option::Some(#msg) },
                None => quote! { ::std::option::Option::None },
            };
            let predicate_tokens = match &ident.predicate {
                Some(sql) => quote! { ::std::option::Option::Some(#sql) },
                None => quote! { ::std::option::Option::None },
            };
            let nils_distinct = ident.nils_distinct;
            quote! {
                ::ash_core::IdentityDef {
                    name: #name_str,
                    keys: &[#(#key_strs),*],
                    message: #msg_tokens,
                    predicate: #predicate_tokens,
                    nils_distinct: #nils_distinct,
                }
            }
        })
        .collect();

    let index_defs: Vec<_> = def
        .indexes
        .iter()
        .map(|index| {
            let name_str = index.name.to_string();
            let key_strs: Vec<String> = index.keys.iter().map(|k| k.to_string()).collect();
            let predicate_tokens = match &index.predicate {
                Some(sql) => quote! { ::std::option::Option::Some(#sql) },
                None => quote! { ::std::option::Option::None },
            };
            let method_tokens = match &index.method {
                Some(method) => quote! { ::std::option::Option::Some(#method) },
                None => quote! { ::std::option::Option::None },
            };
            quote! {
                ::ash_core::IndexDef {
                    name: #name_str,
                    keys: &[#(#key_strs),*],
                    predicate: #predicate_tokens,
                    method: #method_tokens,
                }
            }
        })
        .collect();

    let check_defs: Vec<_> = def
        .checks
        .iter()
        .map(|check| {
            let name_str = check.name.to_string();
            let expression = &check.expression;
            quote! {
                ::ash_core::CheckDef {
                    name: #name_str,
                    expression: #expression,
                }
            }
        })
        .collect();

    let embedded_lit = def.embedded;
    let data_layer_tokens = if def.embedded {
        quote! { ::ash_core::DataLayerKind::Embedded }
    } else if let Some(dl) = &def.data_layer {
        let s = dl.to_string();
        match s.as_str() {
            "postgres" => quote! { ::ash_core::DataLayerKind::Postgres },
            "sqlite" => quote! { ::ash_core::DataLayerKind::Sqlite },
            "memory" => quote! { ::ash_core::DataLayerKind::Memory },
            "embedded" => quote! { ::ash_core::DataLayerKind::Embedded },
            _ => quote! { ::ash_core::DataLayerKind::Custom(#s) },
        }
    } else if let Some(store_ty) = &def.store {
        let s = quote!(#store_ty).to_string().replace(' ', "");
        if s.ends_with("SqliteStore") {
            quote! { ::ash_core::DataLayerKind::Sqlite }
        } else if s.ends_with("PostgresStore") {
            quote! { ::ash_core::DataLayerKind::Postgres }
        } else {
            quote! { ::ash_core::DataLayerKind::Memory }
        }
    } else {
        quote! { ::ash_core::DataLayerKind::Memory }
    };

    let mut identity_methods = Vec::new();
    for ident in &def.identities {
        let name = &ident.name;
        let fn_get = format_ident!("get_by_{}", name);
        let fn_find = format_ident!("find_by_{}", name);

        let mut arg_names = Vec::new();
        let mut arg_tys = Vec::new();
        let mut filter_exprs = Vec::new();

        for key in &ident.keys {
            let attr = def
                .attributes
                .iter()
                .find(|a| a.ident == *key)
                .ok_or_else(|| {
                    Error::new_spanned(
                        key,
                        format!("unknown attribute `{key}` in identity `{name}`"),
                    )
                })?;
            let ty = option_inner(&attr.ty).unwrap_or(&attr.ty);
            arg_names.push(key);
            arg_tys.push(ty);
            filter_exprs.push(quote! {
                Self::#key.eq(#key.into())
            });
        }

        let combined_filter = if filter_exprs.len() == 1 {
            quote! { #(#filter_exprs)* }
        } else {
            let first = &filter_exprs[0];
            let rest = &filter_exprs[1..];
            quote! { #first #(& #rest)* }
        };

        let name_str = name.to_string();
        identity_methods.push(quote! {
            pub const #name: &'static str = #name_str;

            pub async fn #fn_get<D: ::ash_core::DataLayer>(
                ctx: &::ash_core::Context<D>,
                #(#arg_names: impl ::std::convert::Into<#arg_tys>),*
            ) -> ::ash_core::Result<Self> {
                Self::query(ctx)
                    .filter(#combined_filter)
                    .one()
                    .await
            }

            pub async fn #fn_find<D: ::ash_core::DataLayer>(
                ctx: &::ash_core::Context<D>,
                #(#arg_names: impl ::std::convert::Into<#arg_tys>),*
            ) -> ::ash_core::Result<::std::option::Option<Self>> {
                Self::query(ctx)
                    .filter(#combined_filter)
                    .first()
                    .await
            }
        });
    }

    let timestamps_tokens = match &def.timestamps {
        Some(ts) => {
            let c_str = ts.created_at.to_string();
            let u_str = ts.updated_at.to_string();
            quote! { ::std::option::Option::Some((#c_str, #u_str)) }
        }
        None => quote! { ::std::option::Option::None },
    };

    let multitenancy_tokens = match &def.multitenancy {
        Some(mt) => {
            let global = mt.global;
            let strat_tokens = match mt.strategy.as_deref() {
                Some("context") => quote! { ::ash_core::MultitenancyStrategy::Context },
                _ => {
                    let attr = mt.attribute.as_deref().unwrap_or("tenant_id");
                    quote! { ::ash_core::MultitenancyStrategy::Attribute(#attr) }
                }
            };
            quote! {
                ::std::option::Option::Some(::ash_core::MultitenancyDef {
                    strategy: #strat_tokens,
                    global: #global,
                })
            }
        }
        None => quote! { ::std::option::Option::None },
    };

    let (store_type_tokens, store_name_tokens) = if let Some(store_ty) = &def.store {
        (quote! { #store_ty }, quote! { stringify!(#store_ty) })
    } else if let Some(dl) = &def.data_layer {
        let s = dl.to_string();
        match s.as_str() {
            "postgres" => (
                quote! { ::ash_core::PostgresStore },
                quote! { "PostgresStore" },
            ),
            "sqlite" => (quote! { ::ash_core::SqliteStore }, quote! { "SqliteStore" }),
            "memory" => (quote! { ::ash_core::MemoryStore }, quote! { "MemoryStore" }),
            _ => (
                quote! { ::ash_core::DefaultStore },
                quote! { "DefaultStore" },
            ),
        }
    } else {
        (
            quote! { ::ash_core::DefaultStore },
            quote! { "DefaultStore" },
        )
    };

    Ok(quote! {
        #[derive(Clone, Debug, PartialEq, Eq)]
        #(#outer_attrs)*
        pub struct #resource {
            #(#struct_fields,)*
        }

        impl ::ash_core::Resource for #resource {
            type Store = #store_type_tokens;

            const DEF: ::ash_core::ResourceDef = {
                #(#helper_default_fns)*
                const EXTENSIONS: &'static [&'static dyn ::ash_core::ResourceExtension] = &[#(#ext_tokens),*];
                const NOTIFIERS: &'static [&'static dyn ::ash_core::Notifier] = &[#(#notifier_tokens),*];
                fn __ash_store_type_id() -> ::std::any::TypeId {
                    ::std::any::TypeId::of::<#store_type_tokens>()
                }
                ::ash_core::ResourceDef {
                    name: #resource_str,
                    table: #table_str,
                    attributes: &[#(#attr_defs),*],
                    relationships: &[#(#rel_defs),*],
                    actions: Self::ACTIONS,
                    policies: Self::POLICIES,
                    field_policies: Self::FIELD_POLICIES,
                    calculations: &[#(#calc_defs),*],
                    aggregates: &[#(#agg_defs),*],
                    extensions: EXTENSIONS,
                    notifiers: NOTIFIERS,
                    identities: &[#(#ident_defs),*],
                    indexes: &[#(#index_defs),*],
                    checks: &[#(#check_defs),*],
                    embedded: #embedded_lit,
                    data_layer: #data_layer_tokens,
                    timestamps: #timestamps_tokens,
                    store_type_id: __ash_store_type_id,
                    store_name: #store_name_tokens,
                    multitenancy: #multitenancy_tokens,
                }
            };

            fn id(&self) -> ::uuid::Uuid {
                #pk_fn_body
            }

            fn to_fields(&self) -> ::ash_core::FieldMap {
                let mut map = ::ash_core::FieldMap::new();
                #(#to_inserts)*
                map
            }

            fn from_fields(fields: &::ash_core::FieldMap) -> ::ash_core::Result<Self> {
                Ok(Self {
                    #(#from_inits),*
                })
            }

            fn attach(
                &mut self,
                name: &str,
                related: ::std::vec::Vec<::ash_core::FieldMap>,
            ) -> ::ash_core::Result<()> {
                match name {
                    #(#attach_arms)*
                    other => Err(::ash_core::Error::Invalid(::std::format!(
                        "unknown relationship `{other}` on {}",
                        stringify!(#resource)
                    ))),
                }
            }
        }

        pub const #def_const_name: ::ash_core::ResourceDef = <#resource as ::ash_core::Resource>::DEF;

        #[allow(non_upper_case_globals)]
        pub mod fields {
            #[allow(unused_imports)]
            use super::*;
            #(#field_consts)*
        }

        #[allow(non_upper_case_globals)]
        pub mod #fields_mod_name {
            #[allow(unused_imports)]
            use super::*;
            #(#field_consts)*
        }

        #[allow(non_upper_case_globals)]
        impl #resource {
            #(#associated_field_consts)*
            #(#identity_methods)*
        }

        impl ::ash_core::IntoOption<#resource> for #resource {
            fn into_option(self) -> ::std::option::Option<#resource> {
                ::std::option::Option::Some(self)
            }
        }

        impl ::std::convert::From<#resource> for ::ash_core::Value {
            fn from(resource: #resource) -> Self {
                ::ash_core::Value::Map(::ash_core::Resource::to_fields(&resource))
            }
        }

        impl ::std::convert::From<&#resource> for ::ash_core::Value {
            fn from(resource: &#resource) -> Self {
                ::ash_core::Value::Map(::ash_core::Resource::to_fields(resource))
            }
        }
    })
}
