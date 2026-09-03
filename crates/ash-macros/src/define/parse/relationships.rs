use syn::parse::ParseStream;
use syn::{Error, Ident, Result, Token, Type};

use crate::ast_helpers::{last_ident, option_inner, vec_inner};
use crate::define::ast::{OnDeleteSpec, RelType, RelationshipSpec};

pub fn parse_relationships(input: ParseStream) -> Result<Vec<RelationshipSpec>> {
    let mut rels = Vec::new();
    while !input.is_empty() {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let kind_ident: Ident = input.parse()?;
        let kind = match kind_ident.to_string().as_str() {
            "belongs_to" => RelType::BelongsTo,
            "has_many" => RelType::HasMany,
            "many_to_many" => RelType::ManyToMany,
            other => {
                return Err(Error::new_spanned(
                    kind_ident,
                    format!("unknown relationship `{other}`, expected `belongs_to`, `has_many`, or `many_to_many`"),
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
                        let s: syn::LitStr = flags_content.parse()?;
                        fk = Some(s.value());
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
                        let s: syn::LitStr = flags_content.parse()?;
                        source_attribute_on_join_resource = Some(s.value());
                    }
                    "dest_fk" | "destination_attribute_on_join_resource" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let s: syn::LitStr = flags_content.parse()?;
                        destination_attribute_on_join_resource = Some(s.value());
                    }
                    "on_delete" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let val_str = if flags_content.peek(syn::LitStr) {
                            let s: syn::LitStr = flags_content.parse()?;
                            s.value()
                        } else {
                            let id: Ident = flags_content.parse()?;
                            id.to_string()
                        };
                        on_delete = match val_str.as_str() {
                            "cascade" => OnDeleteSpec::Cascade,
                            "nilify" => OnDeleteSpec::Nilify,
                            "restrict" => OnDeleteSpec::Restrict,
                            "nothing" => OnDeleteSpec::Nothing,
                            other => {
                                return Err(Error::new_spanned(
                                    flag_ident,
                                    format!("unknown on_delete value `{other}`, expected `cascade`, `nilify`, `restrict`, or `nothing`"),
                                ));
                            }
                        };
                    }
                    other => {
                        return Err(Error::new_spanned(
                            flag_ident,
                            format!("unknown relationship option `{other}`, expected `fk`, `through`, `source_fk`, `dest_fk`, or `on_delete`"),
                        ));
                    }
                }
                if flags_content.peek(Token![,]) {
                    let _: Token![,] = flags_content.parse()?;
                }
            }
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        let (dest, struct_field_ty) = match kind {
            RelType::BelongsTo => {
                if let Some(inner) = option_inner(&ty) {
                    let dest_ident = last_ident(inner)
                        .ok_or_else(|| {
                            Error::new_spanned(inner, "expected destination type identifier")
                        })?
                        .clone();
                    (dest_ident, syn::parse_quote!(::ash_core::Rel<#ty>))
                } else {
                    let dest_ident = last_ident(&ty)
                        .ok_or_else(|| {
                            Error::new_spanned(&ty, "expected destination type identifier")
                        })?
                        .clone();
                    (
                        dest_ident.clone(),
                        syn::parse_quote!(::ash_core::Rel<::std::option::Option<#dest_ident>>),
                    )
                }
            }
            RelType::HasMany | RelType::ManyToMany => {
                if let Some(inner) = vec_inner(&ty) {
                    let dest_ident = last_ident(inner)
                        .ok_or_else(|| {
                            Error::new_spanned(inner, "expected destination type identifier")
                        })?
                        .clone();
                    (dest_ident, syn::parse_quote!(::ash_core::Rel<#ty>))
                } else {
                    let dest_ident = last_ident(&ty)
                        .ok_or_else(|| {
                            Error::new_spanned(&ty, "expected destination type identifier")
                        })?
                        .clone();
                    (
                        dest_ident.clone(),
                        syn::parse_quote!(::ash_core::Rel<::std::vec::Vec<#dest_ident>>),
                    )
                }
            }
        };

        rels.push(RelationshipSpec {
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
        });
    }
    Ok(rels)
}
