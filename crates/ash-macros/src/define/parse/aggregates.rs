use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::{Error, Ident, Lit, Result, Token, Type};

use crate::define::ast::{AggregateFilterSpec, AggregateKindSpec, AggregateSpec};

use super::helpers::require_semi;

pub fn parse_aggregates(input: ParseStream, errors: &mut Vec<Error>) -> Vec<AggregateSpec> {
    let mut aggs = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_aggregate(&fork, errors) {
            Ok(agg) => {
                input.advance_to(&fork);
                aggs.push(agg);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                super::recover::skip_item(input);
            }
        }
    }
    aggs
}

fn parse_one_aggregate(input: ParseStream, errors: &mut Vec<Error>) -> Result<AggregateSpec> {
    let outer_attrs = input.call(syn::Attribute::parse_outer)?;
    let ident: Ident = input.parse()?;
    let _: Token![:] = input.parse()?;
    let ty: Type = input.parse()?;
    let _: Token![=] = input.parse()?;

    let func: Ident = input.parse()?;
    let content;
    syn::parenthesized!(content in input);

    let func_str = func.to_string();
    let (kind, relationship, filter) = match func_str.as_str() {
        "count" => {
            let relationship: Ident = content.parse()?;
            let filter = parse_optional_aggregate_filter(&content)?;
            (AggregateKindSpec::Count, relationship, filter)
        }
        "exists" => {
            let relationship: Ident = content.parse()?;
            let filter = parse_optional_aggregate_filter(&content)?;
            (AggregateKindSpec::Exists, relationship, filter)
        }
        "first" => {
            let relationship: Ident = content.parse()?;
            let _: Token![,] = content.parse()?;
            let field: Ident = content.parse()?;
            let filter = parse_optional_aggregate_filter(&content)?;
            (AggregateKindSpec::First { field }, relationship, filter)
        }
        "sum" => {
            let relationship: Ident = content.parse()?;
            let _: Token![,] = content.parse()?;
            let field: Ident = content.parse()?;
            let filter = parse_optional_aggregate_filter(&content)?;
            (AggregateKindSpec::Sum { field }, relationship, filter)
        }
        other => {
            return Err(Error::new_spanned(
                func,
                format!(
                    "unknown aggregate function `{other}`, expected `count`, `exists`, `first`, or `sum`"
                ),
            ));
        }
    };

    require_semi(input, errors, "aggregate");

    Ok(AggregateSpec {
        outer_attrs,
        ident,
        ty,
        relationship,
        kind,
        filter,
    })
}

pub fn parse_optional_aggregate_filter(
    content: ParseStream,
) -> Result<Option<AggregateFilterSpec>> {
    if content.is_empty() {
        return Ok(None);
    }
    if content.peek(Token![,]) {
        let _: Token![,] = content.parse()?;
    }
    if content.is_empty() {
        return Ok(None);
    }
    let filter_kw: Ident = content.parse()?;
    if filter_kw != "filter" {
        return Err(Error::new_spanned(filter_kw, "expected `filter:`"));
    }
    if content.peek(Token![:]) {
        let _: Token![:] = content.parse()?;
    } else if content.peek(Token![=]) {
        let _: Token![=] = content.parse()?;
    }
    let field: Ident = content.parse()?;
    let is_ne = if content.peek(Token![!=]) {
        let _: Token![!=] = content.parse()?;
        true
    } else if content.peek(Token![==]) {
        let _: Token![==] = content.parse()?;
        false
    } else if content.peek(Token![=]) {
        let _: Token![=] = content.parse()?;
        false
    } else {
        return Err(Error::new_spanned(
            field,
            "expected `==`, `!=`, or `=` in filter",
        ));
    };
    let value: Lit = content.parse()?;
    if is_ne {
        Ok(Some(AggregateFilterSpec::Ne { field, value }))
    } else {
        Ok(Some(AggregateFilterSpec::Eq { field, value }))
    }
}
