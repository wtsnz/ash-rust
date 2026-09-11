use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::punctuated::Punctuated;
use syn::{Error, Ident, Result, Token};

use crate::define::ast::IdentitySpec;

use super::helpers::require_semi;

pub fn parse_identities(input: ParseStream, errors: &mut Vec<Error>) -> Vec<IdentitySpec> {
    let mut identities = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_identity(&fork, errors) {
            Ok(identity) => {
                input.advance_to(&fork);
                identities.push(identity);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_identity(input);
            }
        }
    }
    identities
}

fn parse_one_identity(input: ParseStream, errors: &mut Vec<Error>) -> Result<IdentitySpec> {
    let ident_kw: Ident = input.parse()?;
    if ident_kw != "identity" {
        return Err(Error::new_spanned(ident_kw, "expected `identity`"));
    }
    let name: Ident = input.parse()?;
    if input.peek(Token![:]) {
        let _: Token![:] = input.parse()?;
    }
    let keys_content;
    syn::bracketed!(keys_content in input);
    let list = Punctuated::<Ident, Token![,]>::parse_terminated(&keys_content)?;
    let keys: Vec<Ident> = list.into_iter().collect();
    let mut message = None;
    if input.peek(Token![,]) {
        let _: Token![,] = input.parse()?;
        if input.peek(Ident) {
            let msg_kw: Ident = input.parse()?;
            if msg_kw == "message" {
                if input.peek(Token![:]) || input.peek(Token![=]) {
                    let _ = input.parse::<proc_macro2::TokenTree>()?;
                }
                let msg_lit: syn::LitStr = input.parse()?;
                message = Some(msg_lit.value());
            }
        }
    }
    require_semi(input, errors, "identity");
    Ok(IdentitySpec {
        name,
        keys,
        message,
    })
}
