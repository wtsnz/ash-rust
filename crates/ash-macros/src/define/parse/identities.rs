use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::{Error, Ident, Result, Token};

use crate::define::ast::IdentitySpec;

pub fn parse_identities(input: ParseStream) -> Result<Vec<IdentitySpec>> {
    let mut identities = Vec::new();
    while !input.is_empty() {
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
        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }
        identities.push(IdentitySpec {
            name,
            keys,
            message,
        });
    }
    Ok(identities)
}
