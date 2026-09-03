use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, Result, Token, Type};

use crate::define::ast::{AttributeSpec, TimestampsSpec};

pub fn parse_attributes(
    input: ParseStream,
) -> Result<(Vec<AttributeSpec>, Option<TimestampsSpec>)> {
    let mut attrs = Vec::new();
    let mut timestamps = None;
    while !input.is_empty() {
        if input.peek(Ident) {
            let fork = input.fork();
            let id: Ident = fork.parse()?;
            if id == "timestamps" {
                let _: Ident = input.parse()?;
                let mut created_at = syn::Ident::new("created_at", proc_macro2::Span::call_site());
                let mut updated_at = syn::Ident::new("updated_at", proc_macro2::Span::call_site());
                if input.peek(syn::token::Bracket) {
                    let names;
                    syn::bracketed!(names in input);
                    let list = Punctuated::<Ident, Token![,]>::parse_terminated(&names)?;
                    let vec: Vec<Ident> = list.into_iter().collect();
                    if vec.len() >= 2 {
                        created_at = vec[0].clone();
                        updated_at = vec[1].clone();
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                timestamps = Some(TimestampsSpec {
                    created_at,
                    updated_at,
                });
                continue;
            }
        }

        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;

        let mut pk = false;
        let mut version = false;
        let mut generated = false;
        let mut default = None;
        let mut default_fn = None;
        let mut atom = None;
        let mut is_enum = false;

        if input.peek(Token![=]) {
            let _: Token![=] = input.parse()?;
            let expr: Expr = input.parse()?;
            default = Some(expr);
        }

        if input.peek(syn::token::Bracket) {
            let flags_content;
            syn::bracketed!(flags_content in input);
            while !flags_content.is_empty() {
                let flag_ident = flags_content.call(Ident::parse_any)?;
                if flag_ident == "pk" {
                    pk = true;
                    generated = true;
                } else if flag_ident == "version" {
                    version = true;
                } else if flag_ident == "generated" {
                    generated = true;
                } else if flag_ident == "enum" || flag_ident == "ash_enum" {
                    is_enum = true;
                } else if flag_ident == "default" {
                    if flags_content.peek(Token![:]) || flags_content.peek(Token![=]) {
                        let _ = flags_content.parse::<proc_macro2::TokenTree>()?;
                    }
                    let expr: Expr = flags_content.parse()?;
                    default = Some(expr);
                } else if flag_ident == "default_fn" {
                    if flags_content.peek(Token![:]) || flags_content.peek(Token![=]) {
                        let _ = flags_content.parse::<proc_macro2::TokenTree>()?;
                    }
                    let p: syn::Path = flags_content.parse()?;
                    default_fn = Some(p);
                } else if flag_ident == "atom" {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    let s: syn::LitStr = flags_content.parse()?;
                    let list: Vec<String> = s
                        .value()
                        .split(',')
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect();
                    atom = Some(list);
                } else {
                    return Err(Error::new_spanned(
                        flag_ident,
                        "expected `pk`, `version`, `generated`, `default`, `default_fn`, `atom`, or `enum`",
                    ));
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

        attrs.push(AttributeSpec {
            outer_attrs,
            ident,
            ty,
            pk,
            version,
            generated,
            atom,
            is_enum,
            default,
            default_fn,
        });
    }
    Ok((attrs, timestamps))
}
