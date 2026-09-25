use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::{Error, Ident, Result, Token, Type};

use crate::ast_helpers::{last_ident, option_inner, vec_inner};
use crate::define::ast::{OnDeleteSpec, RelType, RelationshipSpec};

pub fn parse_relationships(input: ParseStream, errors: &mut Vec<Error>) -> Vec<RelationshipSpec> {
    let mut rels = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_relationship(&fork, errors) {
            Ok(rel) => {
                input.advance_to(&fork);
                rels.push(rel);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_item(input);
            }
        }
    }
    rels
}

fn parse_one_relationship(input: ParseStream, errors: &mut Vec<Error>) -> Result<RelationshipSpec> {
    let outer_attrs = input.call(syn::Attribute::parse_outer)?;
    let kind_ident: Ident = input.parse()?;
    let kind = match kind_ident.to_string().as_str() {
        "belongs_to" => RelType::BelongsTo,
        "has_many" => RelType::HasMany,
        "has_one" => RelType::HasOne,
        "many_to_many" => RelType::ManyToMany,
        _ => {
            const REL_KINDS: &[&str] = &["belongs_to", "has_many", "has_one", "many_to_many"];
            return Err(crate::ast_helpers::unknown_ident_error(
                &kind_ident,
                REL_KINDS,
                "relationship kind",
            ));
        }
    };

    let ident: Ident = input.parse()?;
    let _: Token![:] = input.parse()?;
    let ty: Type = input.parse()?;

    let mut fk = None;
    let mut through = None;
    let mut source_attribute_on_join_resource = None;
    let mut destination_attribute_on_join_resource = None;
    let mut on_delete = OnDeleteSpec::Nothing;
    let mut on_update = OnDeleteSpec::Nothing;

    if input.peek(syn::token::Bracket) {
        let flags_content;
        syn::bracketed!(flags_content in input);
        while !flags_content.is_empty() {
            let flag_ident: Ident = flags_content.parse()?;
            match flag_ident.to_string().as_str() {
                "fk" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    if flags_content.peek(syn::LitStr) {
                        let s: syn::LitStr = flags_content.parse()?;
                        errors.push(Error::new_spanned(
                            &s,
                            "use `fk: field_name`, not a string literal",
                        ));
                        fk = Some(super::helpers::ident_from_string(&s.value(), s.span())?);
                    } else {
                        let id: Ident = flags_content.parse()?;
                        fk = Some(id);
                    }
                }
                "through" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    let th_ident: Ident = flags_content.parse()?;
                    through = Some(th_ident);
                }
                "source_fk" | "source_attribute_on_join_resource" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    if flags_content.peek(syn::LitStr) {
                        let s: syn::LitStr = flags_content.parse()?;
                        errors.push(Error::new_spanned(
                            &s,
                            "use `source_fk: field_name`, not a string literal",
                        ));
                        source_attribute_on_join_resource = Some(s.value());
                    } else {
                        let id: Ident = flags_content.parse()?;
                        source_attribute_on_join_resource = Some(id.to_string());
                    }
                }
                "dest_fk" | "destination_attribute_on_join_resource" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    if flags_content.peek(syn::LitStr) {
                        let s: syn::LitStr = flags_content.parse()?;
                        errors.push(Error::new_spanned(
                            &s,
                            "use `dest_fk: field_name`, not a string literal",
                        ));
                        destination_attribute_on_join_resource = Some(s.value());
                    } else {
                        let id: Ident = flags_content.parse()?;
                        destination_attribute_on_join_resource = Some(id.to_string());
                    }
                }
                "on_delete" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    let val_str = if flags_content.peek(syn::LitStr) {
                        let s: syn::LitStr = flags_content.parse()?;
                        errors.push(Error::new_spanned(
                            &s,
                            "use `on_delete: cascade`, not a string literal",
                        ));
                        s.value()
                    } else {
                        let id: Ident = flags_content.parse()?;
                        id.to_string()
                    };
                    on_delete = parse_referential_action(&flag_ident, &val_str, "on_delete")?;
                }
                "on_update" => {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    let val_str = if flags_content.peek(syn::LitStr) {
                        let s: syn::LitStr = flags_content.parse()?;
                        errors.push(Error::new_spanned(
                            &s,
                            "use `on_update: cascade`, not a string literal",
                        ));
                        s.value()
                    } else {
                        let id: Ident = flags_content.parse()?;
                        id.to_string()
                    };
                    on_update = parse_referential_action(&flag_ident, &val_str, "on_update")?;
                }
                _ => {
                    const REL_OPTIONS: &[&str] = &[
                        "fk",
                        "through",
                        "source_fk",
                        "source_attribute_on_join_resource",
                        "dest_fk",
                        "destination_attribute_on_join_resource",
                        "on_delete",
                        "on_update",
                    ];
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &flag_ident,
                        REL_OPTIONS,
                        "relationship option",
                    ));
                }
            }
            if flags_content.peek(Token![,]) {
                let _: Token![,] = flags_content.parse()?;
            }
        }
    }

    if input.peek(Token![,]) {
        errors.push(Error::new(
            input.span(),
            "use `;` after relationship, not `,`",
        ));
        let _: Token![,] = input.parse()?;
    } else if input.peek(Token![;]) {
        let _: Token![;] = input.parse()?;
    } else {
        errors.push(Error::new(input.span(), "expected `;` after relationship"));
    }

    let (dest, struct_field_ty) = match kind {
        RelType::BelongsTo | RelType::HasOne => {
            let dest_ty = if let Some(inner) = option_inner(&ty) {
                errors.push(Error::new_spanned(
                    &ty,
                    format!(
                        "write `{kind} {ident}: Dest`, not `Option<Dest>` — wrapping is inferred",
                        kind = kind_ident
                    ),
                ));
                inner.clone()
            } else {
                ty.clone()
            };
            let dest_ident = last_ident(&dest_ty)
                .ok_or_else(|| {
                    Error::new_spanned(&dest_ty, "expected destination type identifier")
                })?
                .clone();
            (
                dest_ident,
                syn::parse_quote!(::ash_core::Rel<::std::option::Option<#dest_ty>>),
            )
        }
        RelType::HasMany | RelType::ManyToMany => {
            let dest_ty = if let Some(inner) = vec_inner(&ty) {
                errors.push(Error::new_spanned(
                    &ty,
                    format!(
                        "write `{kind} {ident}: Dest`, not `Vec<Dest>` — wrapping is inferred",
                        kind = kind_ident
                    ),
                ));
                inner.clone()
            } else {
                ty.clone()
            };
            let dest_ident = last_ident(&dest_ty)
                .ok_or_else(|| {
                    Error::new_spanned(&dest_ty, "expected destination type identifier")
                })?
                .clone();
            (
                dest_ident,
                syn::parse_quote!(::ash_core::Rel<::std::vec::Vec<#dest_ty>>),
            )
        }
    };

    Ok(RelationshipSpec {
        outer_attrs,
        kind,
        ident,
        dest,
        struct_field_ty,
        fk,
        through,
        source_attribute_on_join_resource,
        destination_attribute_on_join_resource,
        on_delete,
        on_update,
    })
}

fn parse_referential_action(flag: &Ident, value: &str, name: &str) -> Result<OnDeleteSpec> {
    match value {
        "cascade" => Ok(OnDeleteSpec::Cascade),
        "nilify" => Ok(OnDeleteSpec::Nilify),
        "restrict" => Ok(OnDeleteSpec::Restrict),
        "nothing" => Ok(OnDeleteSpec::Nothing),
        other => Err(Error::new_spanned(
            flag,
            format!(
                "unknown {name} value `{other}`, expected `cascade`, `nilify`, `restrict`, or `nothing`"
            ),
        )),
    }
}
