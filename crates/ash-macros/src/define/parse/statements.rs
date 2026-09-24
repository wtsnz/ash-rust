use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::{Error, Ident, LitStr, Result, Token};

use crate::define::ast::StatementSpec;

use super::helpers::{optional_semi, require_semi};

pub fn parse_statements(input: ParseStream, errors: &mut Vec<Error>) -> Vec<StatementSpec> {
    let mut statements = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_statement(&fork, errors) {
            Ok(statement) => {
                input.advance_to(&fork);
                statements.push(statement);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_statement(input);
            }
        }
    }
    statements
}

fn parse_one_statement(input: ParseStream, errors: &mut Vec<Error>) -> Result<StatementSpec> {
    let statement_kw: Ident = input.parse()?;
    if statement_kw != "statement" {
        return Err(Error::new_spanned(statement_kw, "expected `statement`"));
    }
    let name: Ident = input.parse()?;
    let mut dialects = Vec::new();
    if input.peek(Ident) {
        let only_kw: Ident = input.parse()?;
        if only_kw != "only" {
            return Err(Error::new_spanned(only_kw, "expected `only` or `{`"));
        }
        let dialect: Ident = input.parse()?;
        dialects.push(dialect.to_string());
    }
    let body;
    syn::braced!(body in input);
    let mut up = None;
    let mut down = None;
    while !body.is_empty() {
        if body.peek(Token![;]) || body.peek(Token![,]) {
            let _ = body.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let key: Ident = body.parse()?;
        let sql: LitStr = body.parse()?;
        require_semi(&body, errors, "statement clause");
        match key.to_string().as_str() {
            "up" => up = Some(sql.value()),
            "down" => down = Some(sql.value()),
            other => {
                return Err(Error::new_spanned(
                    key,
                    format!("unknown statement clause `{other}`, expected `up` or `down`"),
                ));
            }
        }
    }
    let Some(up) = up else {
        return Err(Error::new_spanned(name, "statement requires an `up` clause"));
    };
    optional_semi(input)?;
    Ok(StatementSpec {
        name,
        dialects,
        up,
        down: down.unwrap_or_default(),
    })
}
