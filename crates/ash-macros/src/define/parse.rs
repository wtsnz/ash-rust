mod actions;
mod aggregates;
mod attributes;
mod calculations;
mod helpers;
mod identities;
mod policies;
mod relationships;

use super::ast::*;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, Result, Token, Type};

macro_rules! parse_braced {
    ($input:expr, $content:ident) => {
        let $content;
        let _ = syn::braced!($content in $input);
    };
}

impl Parse for ResourceDefinition {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;

        let mut embedded = false;
        let mut header_kw: Ident = input.parse()?;
        if header_kw == "embedded" {
            embedded = true;
            if input.peek(Token![;]) {
                let _: Token![;] = input.parse()?;
            }
            header_kw = input.parse()?;
        }
        if header_kw != "resource" && header_kw != "name" {
            return Err(Error::new_spanned(
                header_kw,
                "expected `resource` or `name`",
            ));
        }
        let resource: Ident = input.parse()?;
        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        let mut table = None;
        let mut attributes = Vec::new();
        let mut relationships = Vec::new();
        let mut calculations = Vec::new();
        let mut aggregates = Vec::new();
        let mut actions = Vec::new();
        let mut policies = Vec::new();
        let mut field_policies = Vec::new();
        let mut extensions = Vec::new();
        let mut notifiers = Vec::new();
        let mut extends = Vec::new();
        let mut optimistic_lock = None;
        let mut identities = Vec::new();
        let mut data_layer = None;
        let mut store = None;
        let mut timestamps = None;

        while !input.is_empty() {
            let section_ident: Ident = input.parse()?;
            if section_ident == "table" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let table_lit: syn::LitStr = input.parse()?;
                table = Some(table_lit.value());
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "attributes" {
                parse_braced!(input, content);
                let (parsed_attrs, attr_ts) = attributes::parse_attributes(&content)?;
                attributes = parsed_attrs;
                if timestamps.is_none() {
                    timestamps = attr_ts;
                }
            } else if section_ident == "relationships" {
                parse_braced!(input, content);
                relationships = relationships::parse_relationships(&content)?;
            } else if section_ident == "calculations" {
                parse_braced!(input, content);
                calculations = calculations::parse_calculations(&content)?;
            } else if section_ident == "aggregates" {
                parse_braced!(input, content);
                aggregates = aggregates::parse_aggregates(&content)?;
            } else if section_ident == "actions" {
                parse_braced!(input, content);
                actions = actions::parse_actions(&content)?;
            } else if section_ident == "policies" {
                parse_braced!(input, content);
                policies = policies::parse_policies(&content)?;
            } else if section_ident == "field_policies" {
                parse_braced!(input, content);
                field_policies = policies::parse_field_policies(&content)?;
            } else if section_ident == "extensions" {
                if input.peek(syn::token::Bracket) {
                    let items;
                    syn::bracketed!(items in input);
                    let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                    for item in list {
                        extensions.push(item);
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced!(input, content);
                    while !content.is_empty() {
                        let expr: Expr = content.parse()?;
                        extensions.push(expr);
                        if content.peek(Token![,]) || content.peek(Token![;]) {
                            let _ = content.parse::<proc_macro2::TokenTree>();
                        }
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "notifiers" {
                if input.peek(syn::token::Bracket) {
                    let items;
                    syn::bracketed!(items in input);
                    let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                    for item in list {
                        notifiers.push(item);
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced!(input, content);
                    while !content.is_empty() {
                        let expr: Expr = content.parse()?;
                        notifiers.push(expr);
                        if content.peek(Token![,]) || content.peek(Token![;]) {
                            let _ = content.parse::<proc_macro2::TokenTree>();
                        }
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "extend" {
                let macro_path: syn::Path = input.parse()?;
                if input.peek(Token![!]) {
                    let _: Token![!] = input.parse()?;
                }
                let content;
                syn::braced!(content in input);
                let tokens: proc_macro2::TokenStream = content.parse()?;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                extends.push(crate::define::ast::ExtendSpec {
                    macro_path,
                    tokens,
                });
            } else if section_ident == "optimistic_lock" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let opt_ident: Ident = input.parse()?;
                optimistic_lock = Some(opt_ident);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "identities" {
                parse_braced!(input, content);
                identities = identities::parse_identities(&content)?;
            } else if section_ident == "embedded" {
                embedded = true;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "data_layer" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let dl_ident: Ident = input.parse()?;
                data_layer = Some(dl_ident);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "store" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let store_ty: Type = input.parse()?;
                store = Some(store_ty);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "timestamps" {
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
                timestamps = Some(crate::define::ast::TimestampsSpec {
                    created_at,
                    updated_at,
                });
            } else {
                return Err(Error::new_spanned(
                    section_ident,
                    "expected `table`, `attributes`, `relationships`, `calculations`, `aggregates`, `actions`, `policies`, `field_policies`, `extensions`, `notifiers`, `extend`, `optimistic_lock`, `identities`, `embedded`, `data_layer`, `store`, or `timestamps`",
                ));
            }
        }

        if let Some(ref ts) = timestamps {
            if !attributes.iter().any(|a| a.ident == ts.created_at) {
                let c_ident = ts.created_at.clone();
                let str_ty: Type = syn::parse_str("String").unwrap();
                attributes.push(AttributeSpec {
                    outer_attrs: Vec::new(),
                    ident: c_ident,
                    ty: str_ty,
                    pk: false,
                    version: false,
                    generated: true,
                    atom: None,
                    is_enum: false,
                    default: None,
                    default_fn: None,
                });
            }
            if !attributes.iter().any(|a| a.ident == ts.updated_at) {
                let u_ident = ts.updated_at.clone();
                let str_ty: Type = syn::parse_str("String").unwrap();
                attributes.push(AttributeSpec {
                    outer_attrs: Vec::new(),
                    ident: u_ident,
                    ty: str_ty,
                    pk: false,
                    version: false,
                    generated: true,
                    atom: None,
                    is_enum: false,
                    default: None,
                    default_fn: None,
                });
            }
        }

        Ok(Self {
            outer_attrs,
            resource,
            table,
            attributes,
            relationships,
            calculations,
            aggregates,
            actions,
            policies,
            field_policies,
            extensions,
            notifiers,
            extends,
            optimistic_lock,
            identities,
            embedded,
            data_layer,
            store,
            timestamps,
        })
    }
}
