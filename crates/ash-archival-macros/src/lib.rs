//! The `#[archival]` transformer behind `ash-archival`.

use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, ItemMacro, Result, Token};

/// Options from the `archive { ... }` block, mirroring AshArchival.
struct ArchiveOptions {
    attribute: Ident,
    exclude_read_actions: Vec<Ident>,
    exclude_destroy_actions: Vec<Ident>,
    archive_related: Vec<Ident>,
    unarchive_action: Option<Ident>,
}

impl Default for ArchiveOptions {
    fn default() -> Self {
        Self {
            attribute: format_ident!("archived_at"),
            exclude_read_actions: Vec::new(),
            exclude_destroy_actions: Vec::new(),
            archive_related: Vec::new(),
            unarchive_action: None,
        }
    }
}

fn parse_ident_list(input: ParseStream) -> Result<Vec<Ident>> {
    if input.peek(Token![:]) {
        let _: Token![:] = input.parse()?;
    }
    let content;
    syn::bracketed!(content in input);
    Ok(Punctuated::<Ident, Token![,]>::parse_terminated(&content)?
        .into_iter()
        .collect())
}

fn parse_single_ident(input: ParseStream) -> Result<Ident> {
    if input.peek(Token![:]) {
        let _: Token![:] = input.parse()?;
    }
    input.parse()
}

impl Parse for ArchiveOptions {
    fn parse(input: ParseStream) -> Result<Self> {
        let mut options = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "attribute" => options.attribute = parse_single_ident(input)?,
                "exclude_read_actions" => options.exclude_read_actions = parse_ident_list(input)?,
                "exclude_destroy_actions" => {
                    options.exclude_destroy_actions = parse_ident_list(input)?
                }
                "archive_related" => options.archive_related = parse_ident_list(input)?,
                "unarchive_action" => options.unarchive_action = Some(parse_single_ident(input)?),
                other => {
                    return Err(Error::new_spanned(
                        &key,
                        format!(
                            "unknown archive option `{other}`, expected `attribute`, \
                             `exclude_read_actions`, `exclude_destroy_actions`, \
                             `archive_related`, or `unarchive_action`"
                        ),
                    ));
                }
            }
            if input.peek(Token![;]) || input.peek(Token![,]) {
                let _ = input.parse::<TokenTree>()?;
            }
        }
        Ok(options)
    }
}

/// One top-level section of the resource body, kept as raw tokens.
struct Section {
    name: Ident,
    tokens: TokenStream2,
    braced: bool,
}

const SECTIONS: &[&str] = &[
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
    "indexes",
    "checks",
    "statements",
    "embedded",
    "data_layer",
    "store",
    "timestamps",
    "multitenancy",
    "actor",
    "archive",
];

struct ResourceInput {
    outer_attrs: Vec<syn::Attribute>,
    name: Ident,
    sections: Vec<Section>,
}

impl Parse for ResourceInput {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let name: Ident = input.parse()?;
        let body;
        syn::braced!(body in input);
        let mut sections = Vec::new();
        while !body.is_empty() {
            let section: Ident = body.parse()?;
            if !SECTIONS.iter().any(|known| section == *known) {
                return Err(Error::new_spanned(
                    &section,
                    format!("#[archival] does not recognize the `{section}` section"),
                ));
            }
            if body.peek(syn::token::Brace) {
                let content;
                syn::braced!(content in body);
                sections.push(Section {
                    name: section,
                    tokens: content.parse()?,
                    braced: true,
                });
                if body.peek(Token![;]) {
                    let _: Token![;] = body.parse()?;
                }
            } else {
                let mut tokens = TokenStream2::new();
                while !body.is_empty() && !body.peek(Token![;]) {
                    tokens.extend([body.parse::<TokenTree>()?]);
                }
                if body.peek(Token![;]) {
                    let _: Token![;] = body.parse()?;
                }
                sections.push(Section {
                    name: section,
                    tokens,
                    braced: false,
                });
            }
        }
        Ok(Self {
            outer_attrs,
            name,
            sections,
        })
    }
}

/// One action, split so items can be appended to its body without losing anything.
struct Action {
    attrs: Vec<syn::Attribute>,
    kind: Ident,
    name: Ident,
    returns: Option<TokenStream2>,
    body: TokenStream2,
}

fn parse_actions(tokens: TokenStream2) -> Result<Vec<Action>> {
    syn::parse::Parser::parse2(
        |input: ParseStream| {
            let mut actions = Vec::new();
            while !input.is_empty() {
                let attrs = input.call(syn::Attribute::parse_outer)?;
                let kind: Ident = input.parse()?;
                let name: Ident = input.parse()?;
                let mut returns = None;
                if input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;
                    let mut ty = TokenStream2::new();
                    while !input.is_empty()
                        && !input.peek(syn::token::Brace)
                        && !input.peek(Token![;])
                    {
                        ty.extend([input.parse::<TokenTree>()?]);
                    }
                    returns = Some(ty);
                }
                let body = if input.peek(syn::token::Brace) {
                    let content;
                    syn::braced!(content in input);
                    content.parse()?
                } else {
                    TokenStream2::new()
                };
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                actions.push(Action {
                    attrs,
                    kind,
                    name,
                    returns,
                    body,
                });
            }
            Ok(actions)
        },
        tokens,
    )
}

fn declares_attribute(tokens: &TokenStream2, attribute: &Ident) -> bool {
    let trees: Vec<TokenTree> = tokens.clone().into_iter().collect();
    trees.windows(2).any(|pair| match pair {
        [TokenTree::Ident(ident), TokenTree::Punct(punct)] => {
            ident == attribute && punct.as_char() == ':'
        }
        _ => false,
    })
}

/// Soft delete for `resource!`, following AshArchival.
///
/// Adds an `archived_at: Option<UtcDateTime>` attribute, filters `archived_at IS NULL`
/// into every read action, and turns every destroy action into a soft destroy that sets
/// `archived_at`. An `archive { ... }` block inside the resource configures it.
#[proc_macro_attribute]
pub fn archival(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return Error::new(
            proc_macro2::Span::call_site(),
            "#[archival] takes no arguments; configure it with an `archive { ... }` block",
        )
        .to_compile_error()
        .into();
    }
    let item: TokenStream2 = item.into();
    let parsed = syn::parse2::<ItemMacro>(item.clone())
        .and_then(|item_macro| syn::parse2::<ResourceInput>(item_macro.mac.tokens))
        .or_else(|_| syn::parse2::<ResourceInput>(item));
    match parsed.and_then(expand) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand(mut input: ResourceInput) -> Result<TokenStream2> {
    let options = match input.sections.iter().position(|s| s.name == "archive") {
        Some(index) => {
            let section = input.sections.remove(index);
            syn::parse2::<ArchiveOptions>(section.tokens)?
        }
        None => ArchiveOptions::default(),
    };
    let attribute = &options.attribute;
    let attribute_str = attribute.to_string();

    // 1. The archive attribute.
    match input.sections.iter_mut().find(|s| s.name == "attributes") {
        Some(section) if declares_attribute(&section.tokens, attribute) => {}
        Some(section) => {
            let existing = &section.tokens;
            section.tokens = quote! {
                #existing
                #attribute: Option<::ash_core::UtcDateTime>;
            };
        }
        None => {
            return Err(Error::new_spanned(
                &input.name,
                "#[archival] needs an `attributes { ... }` section",
            ));
        }
    }

    // 2. Reads hide archived rows; destroys archive instead of deleting.
    let actions_section = input
        .sections
        .iter_mut()
        .find(|s| s.name == "actions")
        .ok_or_else(|| Error::new_spanned(&input.name, "#[archival] needs an `actions` section"))?;
    let mut actions = parse_actions(actions_section.tokens.clone())?;
    for excluded in options
        .exclude_read_actions
        .iter()
        .chain(&options.exclude_destroy_actions)
    {
        if !actions.iter().any(|action| action.name == *excluded) {
            return Err(Error::new_spanned(
                excluded,
                format!("archive excludes `{excluded}`, but there is no such action"),
            ));
        }
    }
    let related = &options.archive_related;
    let cascade = if related.is_empty() {
        quote! {}
    } else {
        quote! { cascade_destroy [#(#related),*]; }
    };
    for action in &mut actions {
        let body = &action.body;
        if action.kind == "read" && !options.exclude_read_actions.contains(&action.name) {
            action.body = quote! {
                #body
                prepare filter(#attribute.is_nil());
            };
        } else if action.kind == "destroy"
            && !options.exclude_destroy_actions.contains(&action.name)
        {
            action.body = quote! {
                #body
                soft;
                #cascade
                change custom(&::ash_archival::ArchiveChange::new(#attribute_str));
            };
        }
    }
    if let Some(unarchive) = &options.unarchive_action {
        let clear = quote! {
            change custom(&::ash_archival::UnarchiveChange::new(#attribute_str));
        };
        match actions.iter_mut().find(|action| action.name == *unarchive) {
            Some(action) if action.kind == "update" => {
                let body = &action.body;
                action.body = quote! { #body #clear };
            }
            Some(action) => {
                return Err(Error::new_spanned(
                    &action.name,
                    "unarchive_action must name an update action",
                ));
            }
            None => actions.push(Action {
                attrs: Vec::new(),
                kind: format_ident!("update"),
                name: unarchive.clone(),
                returns: None,
                body: clear,
            }),
        }
    }
    let rebuilt = actions.iter().map(|action| {
        let Action {
            attrs,
            kind,
            name,
            returns,
            body,
        } = action;
        let returns = returns.as_ref().map(|ty| quote! { , #ty });
        quote! { #(#attrs)* #kind #name #returns { #body } }
    });
    actions_section.tokens = quote! { #(#rebuilt)* };

    // 3. Archive settings for runtime lookups.
    let lits = |idents: &[Ident]| idents.iter().map(Ident::to_string).collect::<Vec<_>>();
    let reads = lits(&options.exclude_read_actions);
    let destroys = lits(&options.exclude_destroy_actions);
    let related_strs = lits(&options.archive_related);
    let unarchive = match &options.unarchive_action {
        Some(name) => {
            let name = name.to_string();
            quote! { ::std::option::Option::Some(#name) }
        }
        None => quote! { ::std::option::Option::None },
    };
    let archive_def = quote! {
        &::ash_archival::ArchiveDef::new(
            #attribute_str,
            &[#(#reads),*],
            &[#(#destroys),*],
            &[#(#related_strs),*],
            #unarchive,
        )
    };
    match input.sections.iter_mut().find(|s| s.name == "extensions") {
        Some(section) if section.braced => {
            let existing = &section.tokens;
            section.tokens = quote! { #archive_def; #existing };
        }
        Some(section) => {
            let inner = match section.tokens.clone().into_iter().next() {
                Some(TokenTree::Group(group)) => group.stream(),
                _ => {
                    return Err(Error::new_spanned(
                        &section.name,
                        "expected `extensions [ ... ]` or `extensions { ... }`",
                    ));
                }
            };
            section.tokens = quote! { #archive_def, #inner };
            section.braced = true;
        }
        None => input.sections.push(Section {
            name: format_ident!("extensions"),
            tokens: quote! { #archive_def; },
            braced: true,
        }),
    }

    let ResourceInput {
        outer_attrs,
        name,
        sections,
    } = input;
    let sections = sections.iter().map(|section| {
        let Section {
            name,
            tokens,
            braced,
        } = section;
        if *braced {
            quote! { #name { #tokens } }
        } else {
            quote! { #name #tokens; }
        }
    });
    Ok(quote! {
        #(#outer_attrs)*
        ::ash_core::resource! {
            #name {
                #(#sections)*
            }
        }

        impl #name {
            /// Whether this record has been archived.
            pub fn is_archived(&self) -> bool {
                self.#attribute.is_some()
            }
        }
    })
}
