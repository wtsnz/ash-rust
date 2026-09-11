use super::ast::{
    CodeInterfaceArg, CodeInterfaceSpec, CodeInterfaceTarget, DomainDefinition, DomainResourceSpec,
};
use crate::ast_helpers::combine_errors;
use proc_macro2::Delimiter;
use syn::parse::{Parse, ParseStream, Parser};
use syn::{Error, Ident, Result, Token, Type};

pub struct ParseResult {
    pub def: DomainDefinition,
    pub errors: Vec<Error>,
}

pub fn parse_domain(tokens: proc_macro2::TokenStream) -> ParseResult {
    let extracted = crate::ast_helpers::extract_domain_ident(&tokens);
    match (|input: ParseStream| {
        let mut errors = Vec::new();
        let def = parse_from_stream(input, &mut errors);
        Ok(ParseResult { def, errors })
    })
    .parse2(tokens)
    {
        Ok(result) => result,
        Err(err) => ParseResult {
            def: DomainDefinition::empty(extracted.unwrap_or_else(invalid_domain_ident)),
            errors: vec![err],
        },
    }
}

impl Parse for DomainDefinition {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut errors = Vec::new();
        let def = parse_from_stream(input, &mut errors);
        if let Some(err) = combine_errors(errors) {
            Err(err)
        } else {
            Ok(def)
        }
    }
}

fn invalid_domain_ident() -> Ident {
    Ident::new("__AshInvalidDomain", proc_macro2::Span::call_site())
}

fn skip_one_tree(input: ParseStream) -> bool {
    input
        .step(|cursor| {
            if let Some((_, rest)) = cursor.token_tree() {
                Ok((true, rest))
            } else {
                Ok((false, *cursor))
            }
        })
        .unwrap_or(false)
}

fn skip_group(input: ParseStream, delimiter: Delimiter) -> bool {
    input
        .step(|cursor| {
            if let Some((_, _, rest)) = cursor.group(delimiter) {
                Ok((true, rest))
            } else {
                Ok((false, *cursor))
            }
        })
        .unwrap_or(false)
}

fn skip_to_semi(input: ParseStream) {
    while !input.is_empty() {
        if input.peek(Token![;]) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if skip_group(input, Delimiter::Brace)
            || skip_group(input, Delimiter::Bracket)
            || skip_group(input, Delimiter::Parenthesis)
        {
            continue;
        }
        if !skip_one_tree(input) {
            return;
        }
    }
}

fn require_semi(input: ParseStream, errors: &mut Vec<Error>, what: &str) {
    if input.peek(Token![;]) {
        let _ = input.parse::<Token![;]>();
        return;
    }
    if input.peek(Token![,]) {
        let comma: Token![,] = match input.parse() {
            Ok(c) => c,
            Err(e) => {
                errors.push(e);
                return;
            }
        };
        errors.push(Error::new(
            comma.span,
            format!("use `;` after {what}, not `,`"),
        ));
        return;
    }
    errors.push(Error::new(
        input.span(),
        format!("expected `;` after {what}"),
    ));
}

fn parse_header(input: ParseStream, errors: &mut Vec<Error>) -> Ident {
    let header: Ident = match input.parse() {
        Ok(id) => id,
        Err(e) => {
            errors.push(e);
            return invalid_domain_ident();
        }
    };

    if header == "domain" || header == "name" {
        errors.push(Error::new_spanned(
            &header,
            "use `Name { ... }`, not `domain Name;`",
        ));
        let name = match input.parse::<Ident>() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                invalid_domain_ident()
            }
        };
        let _ = input.parse::<Token![;]>();
        return name;
    }

    if !input.peek(syn::token::Brace) {
        errors.push(Error::new_spanned(
            &header,
            "expected `Name { ... }` domain body",
        ));
    }

    header
}

fn take_brace<'a>(input: ParseStream<'a>) -> Result<syn::parse::ParseBuffer<'a>> {
    let content;
    syn::braced!(content in input);
    Ok(content)
}

fn with_body_stream(
    input: ParseStream,
    errors: &mut Vec<Error>,
    f: impl FnOnce(ParseStream, &mut Vec<Error>),
) {
    if input.peek(syn::token::Brace) {
        match take_brace(input) {
            Ok(content) => f(&content, errors),
            Err(e) => {
                errors.push(e);
                f(input, errors);
            }
        }
    } else {
        f(input, errors);
    }
}

fn parse_from_stream(input: ParseStream, errors: &mut Vec<Error>) -> DomainDefinition {
    let outer_attrs = match input.call(syn::Attribute::parse_outer) {
        Ok(attrs) => attrs,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    let domain_name = parse_header(input, errors);
    let mut resources = Vec::new();
    with_body_stream(input, errors, |body, errors| {
        parse_sections(body, errors, &mut resources);
    });
    DomainDefinition {
        outer_attrs,
        domain_name,
        resources,
    }
}

fn parse_sections(
    input: ParseStream,
    errors: &mut Vec<Error>,
    resources: &mut Vec<DomainResourceSpec>,
) {
    while !input.is_empty() {
        if input.peek(Token![;]) || input.peek(Token![,]) {
            require_semi(input, errors, "domain item");
            continue;
        }
        let section_kw: Ident = match input.parse() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                skip_to_semi(input);
                continue;
            }
        };
        if section_kw == "resources" {
            if input.peek(syn::token::Brace) {
                match take_brace(input) {
                    Ok(content) => {
                        resources.extend(parse_resources(&content, errors));
                    }
                    Err(e) => errors.push(e),
                }
            } else {
                errors.push(Error::new_spanned(
                    &section_kw,
                    "expected `resources { ... }`",
                ));
                skip_to_semi(input);
            }
        } else {
            errors.push(crate::ast_helpers::unknown_ident_error(
                &section_kw,
                &["resources"],
                "domain section",
            ));
            if input.peek(syn::token::Brace) {
                let _ = skip_group(input, Delimiter::Brace);
            } else {
                skip_to_semi(input);
            }
        }
    }
}

fn parse_resources(input: ParseStream, errors: &mut Vec<Error>) -> Vec<DomainResourceSpec> {
    let mut specs = Vec::new();

    while !input.is_empty() {
        if input.peek(Token![;]) || input.peek(Token![,]) {
            require_semi(input, errors, "resource");
            continue;
        }

        let first: Ident = match input.parse() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                skip_to_semi(input);
                continue;
            }
        };

        let resource_ident = if first == "resource" {
            errors.push(Error::new_spanned(
                &first,
                "use `Ticket;`, not `resource Ticket`",
            ));
            match input.parse() {
                Ok(id) => id,
                Err(e) => {
                    errors.push(e);
                    skip_to_semi(input);
                    continue;
                }
            }
        } else {
            first
        };

        let mut interfaces = Vec::new();
        if input.peek(syn::token::Brace) {
            match take_brace(input) {
                Ok(content) => parse_resource_items(&content, errors, &mut interfaces),
                Err(e) => errors.push(e),
            }
        }

        require_semi(input, errors, "resource");
        specs.push(DomainResourceSpec {
            resource: resource_ident,
            interfaces,
        });
    }

    specs
}

fn parse_resource_items(
    input: ParseStream,
    errors: &mut Vec<Error>,
    interfaces: &mut Vec<CodeInterfaceSpec>,
) {
    while !input.is_empty() {
        if input.peek(Token![;]) || input.peek(Token![,]) {
            require_semi(input, errors, "code interface");
            continue;
        }
        let item_kw: Ident = match input.parse() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                skip_to_semi(input);
                continue;
            }
        };
        if item_kw == "define" {
            match parse_code_interface(input, errors) {
                Ok(spec) => interfaces.push(spec),
                Err(e) => {
                    errors.push(e);
                    skip_to_semi(input);
                }
            }
        } else {
            errors.push(crate::ast_helpers::unknown_ident_error(
                &item_kw,
                &["define"],
                "resource item",
            ));
            skip_to_semi(input);
        }
    }
}

fn parse_code_interface(input: ParseStream, errors: &mut Vec<Error>) -> Result<CodeInterfaceSpec> {
    let fn_name: Ident = input.parse()?;
    if input.peek(Token![,]) {
        let _: Token![,] = input.parse()?;
    }

    let mut action_name = None;
    let mut args = Vec::new();
    let mut get_by = None;
    let mut target = CodeInterfaceTarget::Static;

    while !input.peek(Token![;]) && !input.is_empty() {
        let key: Ident = input.parse()?;
        if input.peek(Token![:]) {
            let _: Token![:] = input.parse()?;
        } else if input.peek(Token![=]) {
            let _: Token![=] = input.parse()?;
        }

        if key == "action" {
            action_name = Some(input.parse()?);
        } else if key == "args" {
            let content;
            syn::bracketed!(content in input);
            while !content.is_empty() {
                let name: Ident = content.parse()?;
                let ty: Type = if content.peek(Token![:]) {
                    let _: Token![:] = content.parse()?;
                    content.parse()?
                } else {
                    syn::parse_quote!(::std::string::String)
                };
                args.push(CodeInterfaceArg { name, ty });
                if content.peek(Token![,]) {
                    let _: Token![,] = content.parse()?;
                }
            }
        } else if key == "get_by" {
            get_by = Some(input.parse()?);
        } else if key == "on" {
            let target_ident: Ident = input.parse()?;
            if target_ident == "record" {
                target = CodeInterfaceTarget::Record;
            } else if target_ident == "id" {
                target = CodeInterfaceTarget::Id;
            } else {
                errors.push(crate::ast_helpers::unknown_ident_error(
                    &target_ident,
                    &["record", "id"],
                    "on target",
                ));
            }
        } else {
            errors.push(crate::ast_helpers::unknown_ident_error(
                &key,
                &["action", "args", "get_by", "on"],
                "code interface option",
            ));
            if input.peek(Token![:]) || input.peek(Token![=]) {
                let _ = skip_one_tree(input);
            }
            if skip_group(input, Delimiter::Bracket)
                || skip_group(input, Delimiter::Brace)
                || skip_group(input, Delimiter::Parenthesis)
            {
            } else if !input.peek(Token![,]) && !input.peek(Token![;]) {
                let _ = skip_one_tree(input);
            }
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        }
    }

    require_semi(input, errors, "code interface");

    let action_name = match action_name {
        Some(name) => name,
        None => {
            errors.push(Error::new_spanned(
                &fn_name,
                "code interface requires `action: <action_name>`",
            ));
            fn_name.clone()
        }
    };

    Ok(CodeInterfaceSpec {
        fn_name,
        action_name,
        args,
        get_by,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::super::ast::DomainDefinition;
    use quote::quote;

    fn parse_err(tokens: proc_macro2::TokenStream) -> syn::Error {
        match syn::parse2::<DomainDefinition>(tokens) {
            Err(e) => e,
            Ok(_) => panic!("expected parse error"),
        }
    }

    #[test]
    fn test_unknown_domain_section_suggests_resources() {
        let err = parse_err(quote! {
            Helpdesk {
                resourcez {
                    Ticket;
                }
            }
        });
        assert!(
            err.to_string().contains("Did you mean `resources`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_resource_item_suggests_define() {
        let err = parse_err(quote! {
            Helpdesk {
                resources {
                    Ticket {
                        defin open_ticket, action: open;
                    };
                }
            }
        });
        assert!(
            err.to_string().contains("Did you mean `define`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_code_interface_key_suggests_correction() {
        let err = parse_err(quote! {
            Helpdesk {
                resources {
                    Ticket {
                        define open_ticket, acton: open;
                    };
                }
            }
        });
        assert!(
            err.to_string().contains("Did you mean `action`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_on_target_suggests_correction() {
        let err = parse_err(quote! {
            Helpdesk {
                resources {
                    Ticket {
                        define close_ticket, action: close, on: recrod;
                    };
                }
            }
        });
        assert!(
            err.to_string().contains("Did you mean `record`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_old_domain_header_is_an_error() {
        let err = parse_err(quote! {
            domain Helpdesk;
            resources {
                Ticket;
            }
        });
        assert!(
            err.to_string()
                .contains("use `Name { ... }`, not `domain Name;`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_comma_after_resource_is_an_error() {
        let err = parse_err(quote! {
            Helpdesk {
                resources {
                    Ticket,
                    Representative;
                }
            }
        });
        assert!(
            err.to_string().contains("use `;` after resource, not `,`"),
            "got: {}",
            err
        );
    }
}
