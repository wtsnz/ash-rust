use syn::parse::ParseStream;
use syn::{Error, Ident, Lit, Result, Token, Type};

use crate::define::ast::{AggregateFilterSpec, AggregateKindSpec, AggregateSpec};

pub fn parse_aggregates(input: ParseStream) -> Result<Vec<AggregateSpec>> {
    let mut aggs = Vec::new();
    while !input.is_empty() {
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

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        aggs.push(AggregateSpec {
            outer_attrs,
            ident,
            ty,
            relationship,
            kind,
            filter,
        });
    }
    Ok(aggs)
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
