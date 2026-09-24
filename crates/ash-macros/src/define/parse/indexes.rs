use syn::parse::discouraged::Speculative;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;
use syn::{Error, Ident, Result, Token};

use crate::define::ast::IndexSpec;

use super::helpers::require_semi;

pub fn parse_indexes(input: ParseStream, errors: &mut Vec<Error>) -> Vec<IndexSpec> {
    let mut indexes = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_index(&fork, errors) {
            Ok(index) => {
                input.advance_to(&fork);
                indexes.push(index);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_index(input);
            }
        }
    }
    indexes
}

fn parse_one_index(input: ParseStream, errors: &mut Vec<Error>) -> Result<IndexSpec> {
    let index_kw: Ident = input.parse()?;
    if index_kw != "index" {
        return Err(Error::new_spanned(index_kw, "expected `index`"));
    }
    let name: Ident = input.parse()?;
    if input.peek(Token![:]) {
        let _: Token![:] = input.parse()?;
    }
    let keys_content;
    syn::bracketed!(keys_content in input);
    let list = Punctuated::<Ident, Token![,]>::parse_terminated(&keys_content)?;
    let keys: Vec<Ident> = list.into_iter().collect();
    require_semi(input, errors, "index");
    Ok(IndexSpec { name, keys })
}
