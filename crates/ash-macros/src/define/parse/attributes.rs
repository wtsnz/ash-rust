use syn::ext::IdentExt;
use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, Result, Token, Type};

use crate::define::ast::{AttributeSpec, TimestampsSpec};

use super::helpers::require_semi;

pub fn parse_attributes(
    input: ParseStream,
    errors: &mut Vec<Error>,
) -> (Vec<AttributeSpec>, Option<TimestampsSpec>) {
    let mut attrs = Vec::new();
    let mut timestamps = None;
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        if input.peek(Ident) {
            let fork = input.fork();
            if let Ok(id) = fork.parse::<Ident>()
                && id == "timestamps"
            {
                timestamps = parse_timestamps_item(input, errors);
                continue;
            }
        }

        let fork = input.fork();
        match parse_one_attribute(&fork, errors) {
            Ok(attr) => {
                input.advance_to(&fork);
                attrs.push(attr);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_item(input);
            }
        }
    }
    (attrs, timestamps)
}

fn parse_timestamps_item(input: ParseStream, errors: &mut Vec<Error>) -> Option<TimestampsSpec> {
    let _: Ident = match input.parse() {
        Ok(id) => id,
        Err(e) => {
            errors.push(e);
            return None;
        }
    };
    let mut created_at = syn::Ident::new("created_at", proc_macro2::Span::call_site());
    let mut updated_at = syn::Ident::new("updated_at", proc_macro2::Span::call_site());
    if input.peek(syn::token::Bracket) {
        let parsed = (|| -> Result<Vec<Ident>> {
            let names;
            syn::bracketed!(names in input);
            let list = Punctuated::<Ident, Token![,]>::parse_terminated(&names)?;
            Ok(list.into_iter().collect())
        })();
        match parsed {
            Ok(vec) => {
                if vec.len() >= 2 {
                    created_at = vec[0].clone();
                    updated_at = vec[1].clone();
                }
            }
            Err(e) => errors.push(e),
        }
    }
    require_semi(input, errors, "`timestamps`");
    Some(TimestampsSpec {
        created_at,
        updated_at,
    })
}

fn parse_one_attribute(input: ParseStream, errors: &mut Vec<Error>) -> Result<AttributeSpec> {
    let outer_attrs = input.call(syn::Attribute::parse_outer)?;
    let ident: Ident = input.parse()?;
    let _: Token![:] = input.parse()?;
    let ty: Type = input.parse()?;

    let mut pk = false;
    let mut version = false;
    let mut generated = false;
    let mut default = None;
    let mut default_fn = None;
    let atom = None;
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
                errors.push(Error::new_spanned(
                    &flag_ident,
                    "use `[enum]` with `#[derive(AshEnum)]`, not `[atom: ...]`",
                ));
                if flags_content.peek(Token![:]) {
                    let _: Token![:] = flags_content.parse()?;
                } else if flags_content.peek(Token![=]) {
                    let _: Token![=] = flags_content.parse()?;
                }
                if flags_content.peek(syn::LitStr) {
                    let _: syn::LitStr = flags_content.parse()?;
                }
            } else {
                const ATTR_OPTIONS: &[&str] = &[
                    "pk",
                    "version",
                    "generated",
                    "default",
                    "default_fn",
                    "enum",
                    "ash_enum",
                ];
                return Err(crate::ast_helpers::unknown_ident_error(
                    &flag_ident,
                    ATTR_OPTIONS,
                    "attribute option",
                ));
            }
            if flags_content.peek(Token![,]) {
                let _: Token![,] = flags_content.parse()?;
            }
        }
    }

    require_semi(input, errors, "attribute");

    Ok(AttributeSpec {
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
    })
}
