use super::ast::{
    CodeInterfaceArg, CodeInterfaceSpec, CodeInterfaceTarget, DomainDefinition, DomainResourceSpec,
};
use syn::parse::{Parse, ParseStream};
use syn::{Error, Ident, Result, Token, Type};

impl Parse for DomainDefinition {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;

        let header_kw: Ident = input.parse()?;
        if header_kw != "domain" && header_kw != "name" {
            return Err(Error::new_spanned(header_kw, "expected `domain` or `name`"));
        }
        if input.peek(Token![:]) {
            let _: Token![:] = input.parse()?;
        }
        let domain_name: Ident = input.parse()?;

        let mut resources = Vec::new();

        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        if input.peek(syn::token::Brace) {
            let body;
            syn::braced!(body in input);
            while !body.is_empty() {
                let section_kw: Ident = body.parse()?;
                if section_kw == "resources" {
                    let r_content;
                    syn::braced!(r_content in body);
                    resources = parse_resources(&r_content)?;
                } else {
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &section_kw,
                        &["resources"],
                        "domain section",
                    ));
                }
            }
        } else {
            while !input.is_empty() {
                let section_kw: Ident = input.parse()?;
                if section_kw == "resources" {
                    let r_content;
                    syn::braced!(r_content in input);
                    resources = parse_resources(&r_content)?;
                } else {
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &section_kw,
                        &["resources"],
                        "domain section",
                    ));
                }
            }
        }

        Ok(Self {
            outer_attrs,
            domain_name,
            resources,
        })
    }
}

fn parse_resources(input: ParseStream) -> Result<Vec<DomainResourceSpec>> {
    let mut specs = Vec::new();

    while !input.is_empty() {
        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
            continue;
        }

        let first: Ident = input.parse()?;
        let resource_ident = if first == "resource" {
            input.parse()?
        } else {
            first
        };

        let mut interfaces = Vec::new();

        if input.peek(syn::token::Brace) {
            let content;
            syn::braced!(content in input);
            while !content.is_empty() {
                let item_kw: Ident = content.parse()?;
                if item_kw == "define" {
                    interfaces.push(parse_code_interface(&content)?);
                } else {
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &item_kw,
                        &["define"],
                        "resource item",
                    ));
                }
            }
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        specs.push(DomainResourceSpec {
            resource: resource_ident,
            interfaces,
        });
    }

    Ok(specs)
}

fn parse_code_interface(input: ParseStream) -> Result<CodeInterfaceSpec> {
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
                return Err(crate::ast_helpers::unknown_ident_error(
                    &target_ident,
                    &["record", "id"],
                    "on target",
                ));
            }
        } else {
            return Err(crate::ast_helpers::unknown_ident_error(
                &key,
                &["action", "args", "get_by", "on"],
                "code interface option",
            ));
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        }
    }

    if input.peek(Token![;]) {
        let _: Token![;] = input.parse()?;
    }

    let action_name = action_name.ok_or_else(|| {
        Error::new_spanned(&fn_name, "code interface requires `action: <action_name>`")
    })?;

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
            domain Helpdesk;
            resourcez {
                Ticket
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
            domain Helpdesk;
            resources {
                Ticket {
                    defin open_ticket, action: open;
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
            domain Helpdesk;
            resources {
                Ticket {
                    define open_ticket, acton: open;
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
            domain Helpdesk;
            resources {
                Ticket {
                    define close_ticket, action: close, on: recrod;
                }
            }
        });
        assert!(
            err.to_string().contains("Did you mean `record`?"),
            "got: {}",
            err
        );
    }
}
