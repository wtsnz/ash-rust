use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::{Error, Ident, LitStr, Result, Token};

use crate::define::ast::CheckSpec;

use super::helpers::require_semi;

pub fn parse_checks(input: ParseStream, errors: &mut Vec<Error>) -> Vec<CheckSpec> {
    let mut checks = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_check(&fork, errors) {
            Ok(check) => {
                input.advance_to(&fork);
                checks.push(check);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_check(input);
            }
        }
    }
    checks
}

fn parse_one_check(input: ParseStream, errors: &mut Vec<Error>) -> Result<CheckSpec> {
    let check_kw: Ident = input.parse()?;
    if check_kw != "check" {
        return Err(Error::new_spanned(check_kw, "expected `check`"));
    }
    let name: Ident = input.parse()?;
    if input.peek(Token![:]) {
        let _: Token![:] = input.parse()?;
    }
    let expression: LitStr = input.parse()?;
    require_semi(input, errors, "check");
    Ok(CheckSpec {
        name,
        expression: expression.value(),
    })
}
