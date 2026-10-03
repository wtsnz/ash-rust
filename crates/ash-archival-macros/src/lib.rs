//! The `#[archival]` transformer behind `ash-archival`.

use ash_macro_support::{Action, ResourceTokens};
use proc_macro::TokenStream;
use proc_macro2::{TokenStream as TokenStream2, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Error, Ident, Result, Token};

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

/// Whether a declared type is `Option<...>`, written plainly or as a path.
fn is_option(ty: &[TokenTree]) -> bool {
    ty.iter()
        .take_while(|tree| !matches!(tree, TokenTree::Punct(punct) if punct.as_char() == '<'))
        .filter_map(|tree| match tree {
            TokenTree::Ident(ident) => Some(ident),
            _ => None,
        })
        .last()
        .is_some_and(|ident| ident == "Option")
}

/// Soft delete for `resource!`, following AshArchival.
///
/// Adds an `archived_at: Option<UtcDateTimeUsec>` attribute, filters `archived_at IS NULL`
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
    match expand_item(item.into()) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

/// Expands `resource! { ... }`, or the bare resource body, with archival added.
fn expand_item(item: TokenStream2) -> Result<TokenStream2> {
    ResourceTokens::from_item(item).and_then(expand)
}

fn expand(mut resource: ResourceTokens) -> Result<TokenStream2> {
    let options = match resource.take_section("archive") {
        Some(section) => syn::parse2::<ArchiveOptions>(section.tokens)?,
        None => ArchiveOptions::default(),
    };
    let attribute = &options.attribute;
    let attribute_str = attribute.to_string();

    // 1. The archive attribute.
    if resource.section("attributes").is_none() {
        return Err(Error::new_spanned(
            &resource.name,
            "#[archival] needs an `attributes { ... }` section",
        ));
    }
    match resource.declared_type(attribute) {
        Some(ty) if !is_option(&ty) => {
            return Err(Error::new_spanned(
                attribute,
                format!(
                    "`{attribute}` is empty until a record is archived, so declare it as \
                     `Option<UtcDateTimeUsec>`"
                ),
            ));
        }
        Some(_) => {}
        None => resource.append_to_section(
            "attributes",
            quote! { #attribute: Option<::ash_core::UtcDateTimeUsec>; },
        ),
    }

    // 2. Reads hide archived rows; destroys archive instead of deleting.
    if resource.section("actions").is_none() {
        return Err(Error::new_spanned(
            &resource.name,
            "#[archival] needs an `actions` section",
        ));
    }
    let mut actions = resource.actions()?;
    let excluded = options
        .exclude_read_actions
        .iter()
        .map(|name| (name, "read"))
        .chain(options.exclude_destroy_actions.iter().map(|name| (name, "destroy")));
    for (excluded, kind) in excluded {
        match actions.iter().find(|action| action.name == *excluded) {
            None => {
                return Err(Error::new_spanned(
                    excluded,
                    format!("archive excludes `{excluded}`, but there is no such action"),
                ));
            }
            Some(action) if action.kind != kind => {
                return Err(Error::new_spanned(
                    excluded,
                    format!(
                        "exclude_{kind}_actions names `{excluded}`, which is a {} action",
                        action.kind
                    ),
                ));
            }
            Some(_) => {}
        }
    }

    let related = &options.archive_related;
    let cascade = if related.is_empty() {
        quote! {}
    } else {
        quote! { cascade_destroy [#(#related),*]; }
    };
    for action in &mut actions {
        if action.is("read") && !options.exclude_read_actions.contains(&action.name) {
            action.append(quote! { prepare filter(#attribute.is_nil()); });
        } else if action.is("destroy") && !options.exclude_destroy_actions.contains(&action.name)
        {
            action.append(quote! {
                soft;
                #cascade
                change custom(&::ash_archival::ArchiveChange::new(#attribute_str));
            });
        }
    }
    if let Some(unarchive) = &options.unarchive_action {
        let clear = quote! {
            change custom(&::ash_archival::UnarchiveChange::new(#attribute_str));
        };
        match actions.iter_mut().find(|action| action.name == *unarchive) {
            Some(action) if action.is("update") => action.append(clear),
            Some(action) => {
                return Err(Error::new_spanned(
                    &action.name,
                    "unarchive_action must name an update action",
                ));
            }
            None => actions.push(Action::new("update", unarchive.clone(), clear)),
        }
    }
    resource.set_actions(&actions);

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
    resource.add_extension(quote! {
        &::ash_archival::ArchiveDef::new(
            #attribute_str,
            &[#(#reads),*],
            &[#(#destroys),*],
            &[#(#related_strs),*],
            #unarchive,
        )
    })?;

    let name = &resource.name;
    let resource_macro = resource.to_resource_macro();
    Ok(quote! {
        #resource_macro

        impl #name {
            /// Whether this record has been archived.
            pub fn is_archived(&self) -> bool {
                self.#attribute.is_some()
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand_err(item: TokenStream2) -> String {
        expand_item(item).unwrap_err().to_string()
    }

    #[test]
    fn carries_attributes_and_accepts_separators_between_actions() {
        let out = expand_item(quote! {
            /// A post.
            #[state_machine]
            resource! {
                Post {
                    attributes { id: Uuid [pk]; }
                    actions {
                        read read { primary; },
                        destroy destroy { primary; };
                    }
                }
            }
        })
        .unwrap()
        .to_string();
        let resource = out.find(":: ash_core :: resource !").unwrap();
        let machine = out.find("# [state_machine]").unwrap();
        let doc = out.find(r#"doc = r" A post.""#).unwrap();
        assert!(machine < resource, "transformers stay on the invocation: {out}");
        assert!(resource < doc, "docs go to the struct: {out}");
        assert!(out.contains("soft ;"), "{out}");
    }

    #[test]
    fn excluded_actions_must_have_the_right_kind() {
        let err = expand_err(quote! {
            Post {
                attributes { id: Uuid [pk]; }
                archive { exclude_read_actions [purge]; }
                actions {
                    read read { primary; }
                    destroy purge;
                }
            }
        });
        assert!(
            err.contains("exclude_read_actions names `purge`, which is a destroy action"),
            "{err}"
        );
    }

    #[test]
    fn the_archive_attribute_must_be_optional() {
        let err = expand_err(quote! {
            Post {
                attributes { id: Uuid [pk]; archived_at: UtcDateTime; }
                actions { read read { primary; } }
            }
        });
        assert!(err.contains("declare it as `Option<UtcDateTimeUsec>`"), "{err}");

        let declared = quote! {
            Post {
                attributes { id: Uuid [pk]; archived_at: ::std::option::Option<UtcDateTime>; }
                actions { read read { primary; } }
            }
        };
        assert!(expand_item(declared).is_ok());
    }
}
