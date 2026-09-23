mod actions;
mod aggregates;
mod attributes;
mod calculations;
mod helpers;
mod identities;
mod policies;
mod recover;
mod relationships;

use super::ast::*;
use crate::ast_helpers::combine_errors;
use syn::parse::{Parse, ParseStream, Parser};
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, Result, Token, Type};

pub struct ParseResult {
    pub def: ResourceDefinition,
    pub errors: Vec<Error>,
}

pub fn parse_resource(tokens: proc_macro2::TokenStream) -> ParseResult {
    let extracted = crate::ast_helpers::extract_resource_ident(&tokens);
    match (|input: ParseStream| {
        let mut errors = Vec::new();
        let def = parse_from_stream(input, &mut errors);
        Ok(ParseResult { def, errors })
    })
    .parse2(tokens)
    {
        Ok(result) => result,
        Err(err) => ParseResult {
            def: ResourceDefinition::empty(extracted.unwrap_or_else(|| {
                Ident::new("__AshInvalidResource", proc_macro2::Span::call_site())
            })),
            errors: vec![err],
        },
    }
}

impl Parse for ResourceDefinition {
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

fn take_brace<'a>(input: ParseStream<'a>) -> Result<syn::parse::ParseBuffer<'a>> {
    let content;
    syn::braced!(content in input);
    Ok(content)
}

fn parse_braced_with<T>(
    input: ParseStream,
    errors: &mut Vec<Error>,
    f: impl FnOnce(ParseStream, &mut Vec<Error>) -> T,
) -> Option<T> {
    match take_brace(input) {
        Ok(content) => Some(f(&content, errors)),
        Err(e) => {
            errors.push(e);
            None
        }
    }
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

fn invalid_resource_ident() -> Ident {
    Ident::new("__AshInvalidResource", proc_macro2::Span::call_site())
}

fn parse_header(input: ParseStream, errors: &mut Vec<Error>) -> (Ident, bool) {
    let mut embedded = false;

    if input.peek(Ident) {
        let fork = input.fork();
        if let Ok(id) = fork.parse::<Ident>()
            && id == "embedded"
        {
            let kw: Ident = match input.parse() {
                Ok(id) => id,
                Err(e) => {
                    errors.push(e);
                    return (invalid_resource_ident(), false);
                }
            };
            if input.peek(Token![;]) {
                errors.push(Error::new_spanned(
                    &kw,
                    "put `embedded` before the resource name: `embedded Name { ... }`",
                ));
                let _ = input.parse::<Token![;]>();
                embedded = true;
            } else {
                embedded = true;
            }
        }
    }

    let header: Ident = match input.parse() {
        Ok(id) => id,
        Err(e) => {
            errors.push(e);
            return (invalid_resource_ident(), embedded);
        }
    };

    if header == "resource" || header == "name" {
        errors.push(Error::new_spanned(
            &header,
            "use `Name { ... }`, not `resource Name;`",
        ));
        let resource = match input.parse::<Ident>() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                invalid_resource_ident()
            }
        };
        let _ = input.parse::<Token![;]>();
        return (resource, embedded);
    }

    if !input.peek(syn::token::Brace) {
        errors.push(Error::new_spanned(
            &header,
            "expected `Name { ... }` resource body",
        ));
    }

    (header, embedded)
}

fn store_type_for_data_layer(dl: &Ident) -> Option<Type> {
    match dl.to_string().as_str() {
        "sqlite" => Some(syn::parse_quote!(::ash_core::SqliteStore)),
        "memory" => Some(syn::parse_quote!(::ash_core::MemoryStore)),
        "postgres" => Some(syn::parse_quote!(::ash_core::PostgresStore)),
        _ => None,
    }
}

fn qualify_builtin_store(ty: Type) -> Type {
    let s = quote::quote!(#ty).to_string().replace(' ', "");
    match s.as_str() {
        "SqliteStore" => syn::parse_quote!(::ash_core::SqliteStore),
        "MemoryStore" => syn::parse_quote!(::ash_core::MemoryStore),
        "PostgresStore" => syn::parse_quote!(::ash_core::PostgresStore),
        _ => ty,
    }
}

fn parse_actor(input: ParseStream, errors: &mut Vec<Error>) -> crate::define::ast::ActorSpec {
    let mut fields = Vec::new();
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let name: Ident = match input.parse() {
            Ok(id) => id,
            Err(e) => {
                errors.push(e);
                recover::skip_item(input);
                continue;
            }
        };
        if input.parse::<Token![:]>().is_err() {
            errors.push(Error::new_spanned(&name, "expected `name: Type;` in actor"));
            recover::skip_item(input);
            continue;
        }
        let ty: Type = match input.parse() {
            Ok(ty) => ty,
            Err(e) => {
                errors.push(e);
                recover::skip_item(input);
                continue;
            }
        };
        helpers::require_semi(input, errors, "actor field");
        fields.push(crate::define::ast::ActorFieldSpec { name, ty });
    }
    crate::define::ast::ActorSpec { fields }
}

fn parse_from_stream(input: ParseStream, errors: &mut Vec<Error>) -> ResourceDefinition {
    let outer_attrs = input.call(syn::Attribute::parse_outer).unwrap_or_else(|e| {
        errors.push(e);
        Vec::new()
    });

    let (resource, mut embedded) = parse_header(input, errors);

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
    let mut actor = None;
    let warnings = Vec::new();

    with_body_stream(input, errors, |input, errors| {
        while !input.is_empty() {
            let section_ident: Ident = match input.parse() {
                Ok(id) => id,
                Err(e) => {
                    errors.push(e);
                    recover::skip_until_section_or_end(input);
                    continue;
                }
            };
            if section_ident == "table" {
                if input.peek(Token![:]) {
                    let _ = input.parse::<Token![:]>();
                }
                match input.parse::<syn::LitStr>() {
                    Ok(table_lit) => table = Some(table_lit.value()),
                    Err(e) => {
                        errors.push(e);
                        recover::skip_to_semi(input);
                    }
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "attributes" {
                if let Some((parsed_attrs, attr_ts)) =
                    parse_braced_with(input, errors, attributes::parse_attributes)
                {
                    attributes = parsed_attrs;
                    if timestamps.is_none() {
                        timestamps = attr_ts;
                    }
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "relationships" {
                if let Some(parsed) =
                    parse_braced_with(input, errors, relationships::parse_relationships)
                {
                    relationships = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "calculations" {
                if let Some(parsed) =
                    parse_braced_with(input, errors, calculations::parse_calculations)
                {
                    calculations = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "aggregates" {
                if let Some(parsed) = parse_braced_with(input, errors, aggregates::parse_aggregates)
                {
                    aggregates = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "actions" {
                if let Some(parsed) = parse_braced_with(input, errors, actions::parse_actions) {
                    actions = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "policies" {
                if let Some(parsed) = parse_braced_with(input, errors, policies::parse_policies) {
                    policies = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "field_policies" {
                if let Some(parsed) =
                    parse_braced_with(input, errors, policies::parse_field_policies)
                {
                    field_policies = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "extensions" {
                if input.peek(syn::token::Bracket) {
                    match (|| -> Result<()> {
                        let items;
                        syn::bracketed!(items in input);
                        let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                        for item in list {
                            extensions.push(item);
                        }
                        Ok(())
                    })() {
                        Ok(()) => {}
                        Err(e) => errors.push(e),
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced_with(input, errors, |content, errors| {
                        while !content.is_empty() {
                            match content.parse::<Expr>() {
                                Ok(expr) => extensions.push(expr),
                                Err(e) => {
                                    errors.push(e);
                                    recover::skip_item(content);
                                }
                            }
                            if content.peek(Token![,]) || content.peek(Token![;]) {
                                let _ = content.parse::<proc_macro2::TokenTree>();
                            }
                        }
                    });
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "notifiers" {
                if input.peek(syn::token::Bracket) {
                    match (|| -> Result<()> {
                        let items;
                        syn::bracketed!(items in input);
                        let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                        for item in list {
                            notifiers.push(item);
                        }
                        Ok(())
                    })() {
                        Ok(()) => {}
                        Err(e) => errors.push(e),
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced_with(input, errors, |content, errors| {
                        while !content.is_empty() {
                            match content.parse::<Expr>() {
                                Ok(expr) => notifiers.push(expr),
                                Err(e) => {
                                    errors.push(e);
                                    recover::skip_item(content);
                                }
                            }
                            if content.peek(Token![,]) || content.peek(Token![;]) {
                                let _ = content.parse::<proc_macro2::TokenTree>();
                            }
                        }
                    });
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "extend" {
                match (|| -> Result<()> {
                    let macro_path: syn::Path = input.parse()?;
                    if input.peek(Token![!]) {
                        let _: Token![!] = input.parse()?;
                    }
                    let content;
                    syn::braced!(content in input);
                    let tokens: proc_macro2::TokenStream = content.parse()?;
                    let _ = input.parse::<Token![;]>();
                    extends.push(crate::define::ast::ExtendSpec { macro_path, tokens });
                    Ok(())
                })() {
                    Ok(()) => {}
                    Err(e) => {
                        errors.push(e);
                        recover::skip_section_body(input);
                    }
                }
            } else if section_ident == "optimistic_lock" {
                if input.peek(Token![:]) {
                    let _ = input.parse::<Token![:]>();
                }
                match input.parse::<Ident>() {
                    Ok(opt_ident) => optimistic_lock = Some(opt_ident),
                    Err(e) => {
                        errors.push(e);
                        recover::skip_to_semi(input);
                    }
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "identities" {
                if let Some(parsed) = parse_braced_with(input, errors, identities::parse_identities)
                {
                    identities = parsed;
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "actor" {
                if let Some(parsed) = parse_braced_with(input, errors, parse_actor) {
                    actor = Some(parsed);
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "embedded" {
                errors.push(Error::new_spanned(
                    &section_ident,
                    "put `embedded` on the resource header: `embedded Name { ... }`",
                ));
                embedded = true;
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "data_layer" {
                errors.push(Error::new_spanned(
                &section_ident,
                "use `store SqliteStore;` (or `MemoryStore` / `PostgresStore`), not `data_layer sqlite`",
            ));
                if input.peek(Token![:]) {
                    let _ = input.parse::<Token![:]>();
                }
                match input.parse::<Ident>() {
                    Ok(dl_ident) => {
                        if store.is_none() {
                            store = store_type_for_data_layer(&dl_ident);
                        }
                        data_layer = Some(dl_ident);
                    }
                    Err(e) => {
                        errors.push(e);
                        recover::skip_to_semi(input);
                    }
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "store" {
                if input.peek(Token![:]) {
                    let _ = input.parse::<Token![:]>();
                }
                match input.parse::<Type>() {
                    Ok(store_ty) => store = Some(qualify_builtin_store(store_ty)),
                    Err(e) => {
                        errors.push(e);
                        recover::skip_to_semi(input);
                    }
                }
                let _ = input.parse::<Token![;]>();
            } else if section_ident == "multitenancy" {
                if let Some(parsed) = parse_braced_with(input, errors, |content, errors| {
                    let mut attribute = None;
                    let mut strategy = None;
                    let mut global = false;
                    while !content.is_empty() {
                        let key_ident: Ident = match content.parse() {
                            Ok(id) => id,
                            Err(e) => {
                                errors.push(e);
                                recover::skip_item(content);
                                continue;
                            }
                        };
                        if content.peek(Token![:]) {
                            let _ = content.parse::<Token![:]>();
                        }
                        if key_ident == "attribute" {
                            if content.peek(syn::LitStr) {
                                if let Ok(lit) = content.parse::<syn::LitStr>() {
                                    attribute = Some(lit.value());
                                }
                            } else if let Ok(attr_ident) = content.parse::<Ident>() {
                                attribute = Some(attr_ident.to_string());
                            }
                        } else if key_ident == "strategy" {
                            if content.peek(syn::LitStr) {
                                if let Ok(lit) = content.parse::<syn::LitStr>() {
                                    strategy = Some(lit.value());
                                }
                            } else if let Ok(strat_ident) = content.parse::<Ident>() {
                                strategy = Some(strat_ident.to_string());
                            }
                        } else if key_ident == "global"
                            && let Ok(lit) = content.parse::<syn::LitBool>() {
                                global = lit.value;
                            }
                        if content.peek(Token![,]) || content.peek(Token![;]) {
                            let _ = content.parse::<proc_macro2::TokenTree>();
                        }
                    }
                    crate::define::ast::MultitenancySpec {
                        attribute,
                        strategy,
                        global,
                    }
                }) {
                    multitenancy = Some(parsed);
                }
                let _ = helpers::optional_semi(input);
            } else if section_ident == "timestamps" {
                let mut created_at = syn::Ident::new("created_at", proc_macro2::Span::call_site());
                let mut updated_at = syn::Ident::new("updated_at", proc_macro2::Span::call_site());
                if input.peek(syn::token::Bracket) {
                    match (|| -> Result<()> {
                        let names;
                        syn::bracketed!(names in input);
                        let list = Punctuated::<Ident, Token![,]>::parse_terminated(&names)?;
                        let vec: Vec<Ident> = list.into_iter().collect();
                        if vec.len() >= 2 {
                            created_at = vec[0].clone();
                            updated_at = vec[1].clone();
                        }
                        Ok(())
                    })() {
                        Ok(()) => {}
                        Err(e) => errors.push(e),
                    }
                }
                let _ = input.parse::<Token![;]>();
                timestamps = Some(crate::define::ast::TimestampsSpec {
                    created_at,
                    updated_at,
                });
            } else {
                errors.push(crate::ast_helpers::unknown_ident_error(
                    &section_ident,
                    recover::SECTION_NAMES,
                    "resource section",
                ));
                recover::skip_section_body(input);
            }
        }
    });

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

    ResourceDefinition {
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
        actor,
        warnings,
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
            TestResource {
            action {
                create create;
            }
        }};
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
            TestResource {
            actions {
                creat add;
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validats [present(title)];
                }
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validate presence(title);
                }
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validate string_length(title, min: 10, max: 2);
                }
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validate numericality(age, min: 100, max: 18);
                }
            }
        }};
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("min (100) cannot be greater than max (18)"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_removed_plural_syntax_is_an_error() {
        let tokens = quote! {
            TestResource {
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
        }};
        let parsed = parse_resource(tokens);
        let msg: String = parsed
            .errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            msg.contains("arguments { ... }"),
            "missing arguments error: {msg}"
        );
        assert!(
            msg.contains("validations [...]"),
            "missing validations error: {msg}"
        );
        assert!(
            msg.contains("changes [...]"),
            "missing changes error: {msg}"
        );
    }

    #[test]
    fn test_crud_brace_accept_is_an_error() {
        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            actions {
                create open {
                    accept {
                        subject: String,
                    }
                }
            }
        }};
        let err = parse_err(tokens);
        let msg = err.to_string();
        assert!(
            msg.contains("accept [field, ...]"),
            "missing CRUD accept error: {msg}"
        );
    }

    #[test]
    fn test_action_keyword_is_an_error() {
        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            actions {
                action summarize {
                    argument notes: String;
                }
            }
        }};
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("use `generic`, not `action`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_string_fk_is_an_error() {
        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                author_id: Uuid;
            }
            relationships {
                belongs_to author: User [fk: "author_id"];
            }
        }};
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("use `fk: field_name`, not a string literal"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_option_relationship_dest_is_an_error() {
        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                author_id: Uuid;
            }
            relationships {
                belongs_to author: Option<User> [fk: author_id];
            }
        }};
        let err = parse_err(tokens);
        let msg = err.to_string();
        assert!(
            msg.contains("not `Option<Dest>`"),
            "missing Option unwrap error: {msg}"
        );
    }

    #[test]
    fn test_vec_relationship_dest_is_an_error() {
        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
            }
            relationships {
                has_many comments: Vec<Comment>;
            }
        }};
        let err = parse_err(tokens);
        let msg = err.to_string();
        assert!(
            msg.contains("not `Vec<Dest>`"),
            "missing Vec unwrap error: {msg}"
        );
    }

    #[test]
    fn test_quoted_calculation_string_is_an_error() {
        use crate::define::ast::CalculationExprSpec;

        let tokens = quote! {
            TestResource {
            attributes {
                id: Uuid [pk];
                subject: String;
            }
            calculations {
                subject_len: i64 = "string_length(subject)";
            }
        }};
        let parsed = parse_resource(tokens);
        assert!(
            parsed
                .errors
                .iter()
                .any(|e| e.to_string().contains("unquoted `string_length(subject)`")),
            "missing quoted calc error: {:?}",
            parsed.errors
        );
        match &parsed.def.calculations[0].expr {
            CalculationExprSpec::StringLength(field) => {
                assert_eq!(field.to_string(), "subject");
            }
            other => panic!("expected StringLength ident, got {other:?}"),
        }
    }

    #[test]
    fn test_unknown_relationship_kind_typo_suggests_correction() {
        let tokens = quote! {
            TestResource {
            relationships {
                belongs_too author: User [fk: author_id];
            }
        }};
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
            TestResource {
            relationships {
                belongs_to author: User [fkey: author_id];
            }
        }};
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
            TestResource {
            attributes {
                id: Uuid [pkk];
            }
        }};
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
            TestResource {
            policies {
                polisy always {
                    authorize_if always;
                }
            }
        }};
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
            TestResource {
            policies {
                policy always {
                    authorize_iff always;
                }
            }
        }};
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
            TestResource {
            policies {
                policy alwayz {
                    authorize_if always;
                }
            }
        }};
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
            TestResource {
            policies {
                policy always {
                    authorize_if actor_eqq(author_id);
                }
            }
        }};
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
            TestResource {
            attributes {
                id: Uuid [pk];
                title: String;
                status: String;
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
        }};
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
    fn test_vacuous_numericality_fails() {
        let tokens = quote! {
            TestResource {
            actions {
                create open {
                    validate numericality(age);
                }
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validate string_length(title);
                }
            }
        }};
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
            TestResource {
            actions {
                create open {
                    validate one_of(status, []);
                }
            }
        }};
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
            TestResource {
            attributes {
                id: Uuid [pk];
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
        }};
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

    #[test]
    fn test_old_resource_header_is_an_error() {
        let tokens = quote! {
            resource TestResource;
            attributes {
                id: Uuid [pk];
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("use `Name { ... }`, not `resource Name;`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_atom_flag_is_an_error() {
        let tokens = quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    status: String [atom: "open,closed"];
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("use `[enum]` with `#[derive(AshEnum)]`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_primary_true_is_an_error() {
        let tokens = quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                }
                actions {
                    read read { primary true; }
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("use `primary;`, not `primary true`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_min_equals_is_an_error() {
        let tokens = quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    title: String;
                }
                actions {
                    create open {
                        validate string_length(title, min = 2);
                    }
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string().contains("use `min: N`, not `min = N`"),
            "got: {}",
            err
        );
    }

    #[test]
    fn test_set_attribute_is_an_error() {
        let tokens = quote! {
            TestResource {
                attributes {
                    id: Uuid [pk];
                    status: String;
                }
                actions {
                    create open {
                        accept [status];
                        change set_attribute(status = "open");
                    }
                }
            }
        };
        let err = parse_err(tokens);
        assert!(
            err.to_string()
                .contains("use `set(...)`, not `set_attribute(...)`"),
            "got: {}",
            err
        );
    }
}
