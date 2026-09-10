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
        let mut multitenancy = None;
        let mut warnings = Vec::new();

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
                helpers::optional_semi(input)?;
            } else if section_ident == "relationships" {
                parse_braced!(input, content);
                relationships = relationships::parse_relationships(&content)?;
                helpers::optional_semi(input)?;
            } else if section_ident == "calculations" {
                parse_braced!(input, content);
                calculations = calculations::parse_calculations(&content, &mut warnings)?;
                helpers::optional_semi(input)?;
            } else if section_ident == "aggregates" {
                parse_braced!(input, content);
                aggregates = aggregates::parse_aggregates(&content)?;
                helpers::optional_semi(input)?;
            } else if section_ident == "actions" {
                parse_braced!(input, content);
                actions = actions::parse_actions(&content, &mut warnings)?;
                helpers::optional_semi(input)?;
            } else if section_ident == "policies" {
                parse_braced!(input, content);
                policies = policies::parse_policies(&content)?;
                helpers::optional_semi(input)?;
            } else if section_ident == "field_policies" {
                parse_braced!(input, content);
                field_policies = policies::parse_field_policies(&content)?;
                helpers::optional_semi(input)?;
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
                extends.push(crate::define::ast::ExtendSpec { macro_path, tokens });
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
                helpers::optional_semi(input)?;
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
            } else if section_ident == "multitenancy" {
                parse_braced!(input, content);
                let mut attribute = None;
                let mut strategy = None;
                let mut global = false;
                while !content.is_empty() {
                    let key_ident: Ident = content.parse()?;
                    if content.peek(Token![:]) {
                        let _: Token![:] = content.parse()?;
                    }
                    if key_ident == "attribute" {
                        if content.peek(syn::LitStr) {
                            let lit: syn::LitStr = content.parse()?;
                            attribute = Some(lit.value());
                        } else {
                            let attr_ident: Ident = content.parse()?;
                            attribute = Some(attr_ident.to_string());
                        }
                    } else if key_ident == "strategy" {
                        if content.peek(syn::LitStr) {
                            let lit: syn::LitStr = content.parse()?;
                            strategy = Some(lit.value());
                        } else {
                            let strat_ident: Ident = content.parse()?;
                            strategy = Some(strat_ident.to_string());
                        }
                    } else if key_ident == "global" {
                        let lit: syn::LitBool = content.parse()?;
                        global = lit.value;
                    }
                    if content.peek(Token![,]) {
                        let _: Token![,] = content.parse()?;
                    } else if content.peek(Token![;]) {
                        let _: Token![;] = content.parse()?;
                    }
                }
                helpers::optional_semi(input)?;
                multitenancy = Some(crate::define::ast::MultitenancySpec {
                    attribute,
                    strategy,
                    global,
                });
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
                const SECTION_NAMES: &[&str] = &[
                    "table",
                    "attributes",
                    "relationships",
                    "calculations",
                    "aggregates",
                    "actions",
                    "policies",
                    "field_policies",
                    "extensions",
                    "notifiers",
                    "extend",
                    "optimistic_lock",
                    "identities",
                    "embedded",
                    "data_layer",
                    "store",
                    "timestamps",
                ];
                return Err(crate::ast_helpers::unknown_ident_error(
                    &section_ident,
                    SECTION_NAMES,
                    "resource section",
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
            multitenancy,
            warnings,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn parse_err(tokens: proc_macro2::TokenStream) -> syn::Error {
        match syn::parse2::<ResourceDefinition>(tokens) {
            Err(e) => e,
            Ok(_) => panic!("expected parse error"),
        }
    }

    #[test]
    fn test_unknown_section_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            action {
                create create;
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `actions`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_action_kind_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            actions {
                creat add;
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `create`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_action_item_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validats [present(title)];
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `validate`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_validation_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate presence(title);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `present`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_string_length_min_greater_than_max_fails() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate string_length(title, min = 10, max = 2);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("min (10) cannot be greater than max (2)"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_numericality_min_greater_than_max_fails() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate numericality(age, min = 100, max = 18);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("min (100) cannot be greater than max (18)"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_deprecated_syntax_emits_synthetic_warnings() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    arguments {
                        title: String,
                    }
                    validations [
                        present(title),
                    ]
                    changes [
                        set(status = "open"),
                    ]
                }
            }
        };
        let def = match syn::parse2::<ResourceDefinition>(tokens) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {}", e),
        };
        assert_eq!(def.warnings.len(), 3);
        let warnings = &def.warnings;
        let warn_str = quote! { #(#warnings)* }.to_string();
        assert!(warn_str.contains("arguments { ... }"));
        assert!(warn_str.contains("validations [...]"));
        assert!(warn_str.contains("changes [...]"));
    }

    #[test]
    fn test_unknown_relationship_kind_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            relationships {
                belongs_too author: User [fk: author_id];
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `belongs_to`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_relationship_option_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            relationships {
                belongs_to author: User [fkey: author_id];
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `fk`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_attribute_option_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pkk];
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `pk`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_policy_declaration_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            policies {
                polisy always {
                    authorize_if always;
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `policy`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_policy_statement_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            policies {
                policy always {
                    authorize_iff always;
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `authorize_if`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_policy_when_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            policies {
                policy alwayz {
                    authorize_if always;
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `always`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_unknown_policy_check_typo_suggests_correction() {
        let tokens = quote! {
            resource TestResource;
            policies {
                policy always {
                    authorize_if actor_eqq(author_id);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("Did you mean `actor_eq`?"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_permissive_trailing_punctuation_and_empty_sections() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                title: String,
                status: String,
            };
            relationships {};
            policies {};
            field_policies {};
            calculations {};
            aggregates {};
            identities {
                identity by_title: [title,];
            };
            actions {
                create open {
                    primary;
                    accept [title,];
                    validate one_of(status, ["open", "closed",]);
                };
                read read {
                    primary;
                };
            };
        };
        let def = match syn::parse2::<ResourceDefinition>(tokens) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {e}"),
        };
        assert_eq!(def.attributes.len(), 3);
        assert!(def.relationships.is_empty());
        assert!(def.policies.is_empty());
        assert!(def.field_policies.is_empty());
        assert!(def.calculations.is_empty());
        assert!(def.aggregates.is_empty());
        assert_eq!(def.identities.len(), 1);
        assert_eq!(def.identities[0].keys.len(), 1);
        assert_eq!(def.identities[0].keys[0].to_string(), "title");
        assert_eq!(def.actions.len(), 2);
        assert_eq!(def.actions[0].accept.len(), 1);
        assert_eq!(def.actions[0].validations.len(), 1);
    }

    #[test]
    fn test_quoted_calculation_string_emits_deprecation_warning() {
        use crate::define::ast::CalculationExprSpec;

        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
                subject: String,
            }
            calculations {
                subject_len: i64 = "string_length(subject)";
            }
        };
        let def = match syn::parse2::<ResourceDefinition>(tokens) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {e}"),
        };
        assert_eq!(def.warnings.len(), 1);
        let warnings = &def.warnings;
        let warn_str = quote! { #(#warnings)* }.to_string();
        assert!(
            warn_str.contains("string_length(subject)"),
            "missing quoted calc deprecation: {warn_str}"
        );
        match &def.calculations[0].expr {
            CalculationExprSpec::StringLength(field) => {
                assert_eq!(field.to_string(), "subject");
            }
            other => panic!("expected StringLength ident, got {other:?}"),
        }
    }

    #[test]
    fn test_vacuous_numericality_fails() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate numericality(age);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains(
                "numericality validation for 'age' must specify at least one of 'min' or 'max'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_vacuous_string_length_fails() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate string_length(title);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains(
                "string_length validation for 'title' must specify at least one of 'min' or 'max'"
            ),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_empty_one_of_fails() {
        let tokens = quote! {
            resource TestResource;
            actions {
                create open {
                    validate one_of(status, []);
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("one_of validation for 'status' must specify at least one allowed value"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_argument_doc_comments_are_parsed() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk],
            }
            actions {
                create open {
                    /// Caller-supplied reason
                    argument reason: String;
                }
            }
            calculations {
                titled(
                    /// Prefix to prepend
                    prefix: String
                ): String = concat(arg(prefix), "x");
            }
        };
        let def = match syn::parse2::<ResourceDefinition>(tokens) {
            Ok(d) => d,
            Err(e) => panic!("parse failed: {e}"),
        };
        assert_eq!(def.actions[0].arguments.len(), 1);
        assert!(
            !def.actions[0].arguments[0].outer_attrs.is_empty(),
            "missing action argument docs"
        );
        assert_eq!(def.calculations[0].arguments.len(), 1);
        assert!(
            !def.calculations[0].arguments[0].outer_attrs.is_empty(),
            "missing calculation argument docs"
        );
    }
}
