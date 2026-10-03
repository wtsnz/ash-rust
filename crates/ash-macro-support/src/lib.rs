//! Shared parsing for `resource!` transformers such as `#[state_machine]`,
//! `#[authentication]`, and `#[archival]`.
//!
//! A transformer wraps `resource! { ... }`, rewrites some of its sections, and emits a
//! plain `::ash_core::resource!` call. [`ResourceTokens`] keeps every section as raw
//! tokens so nothing the transformer does not touch is lost or reformatted, and
//! [`Action`] splits the `actions` section without dropping doc comments, return types,
//! or `read name;` short forms. `ash-macros` validates the rebuilt resource as usual.

use proc_macro2::{TokenStream, TokenTree};
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream, Parser};
use syn::{Attribute, Error, Ident, ItemMacro, Result, Token};

/// One top-level section of the resource body, such as `attributes { ... }` or
/// `table "posts";`.
pub struct Section {
    pub name: Ident,
    pub tokens: TokenStream,
    /// `name { tokens }` when true, `name tokens;` when false.
    pub braced: bool,
}

/// A `resource!` invocation split into sections.
pub struct ResourceTokens {
    /// Attributes on the macro call, such as another transformer or `cfg`. They stay on
    /// the rebuilt call so stacked transformers still run.
    pub call_attrs: Vec<Attribute>,
    /// Attributes inside the braces, before the name. `resource!` puts them on the struct.
    pub outer_attrs: Vec<Attribute>,
    pub embedded: bool,
    pub name: Ident,
    pub sections: Vec<Section>,
}

impl Parse for ResourceTokens {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(Attribute::parse_outer)?;
        let mut embedded = false;
        let fork = input.fork();
        if fork.parse::<Ident>().is_ok_and(|id| id == "embedded") {
            let _: Ident = input.parse()?;
            embedded = true;
        }
        let name: Ident = input.parse()?;
        if !input.peek(syn::token::Brace) {
            return Err(Error::new_spanned(&name, "expected `Name { ... }` resource body"));
        }
        let body;
        syn::braced!(body in input);
        let mut sections = Vec::new();
        while !body.is_empty() {
            let section: Ident = body.parse()?;
            if body.peek(syn::token::Brace) {
                // `resource!` keeps only the last of these, so a transformer editing the
                // first would change nothing.
                let repeatable = section == "extensions" || section == "notifiers";
                if !repeatable && sections.iter().any(|s: &Section| s.braced && s.name == section) {
                    return Err(Error::new_spanned(
                        &section,
                        format!("`{section}` appears twice; merge them into one section"),
                    ));
                }
                let content;
                syn::braced!(content in body);
                sections.push(Section {
                    name: section,
                    tokens: content.parse()?,
                    braced: true,
                });
            } else {
                let tokens = parse_one_line(&section, &body)?;
                sections.push(Section {
                    name: section,
                    tokens,
                    braced: false,
                });
            }
            if body.peek(Token![;]) {
                let _: Token![;] = body.parse()?;
            }
        }
        Ok(Self {
            call_attrs: Vec::new(),
            outer_attrs,
            embedded,
            name,
            sections,
        })
    }
}

/// Section names `resource!` and the transformers understand.
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
    "authentication",
    "state_machine",
];

/// A section written on one line, read with the grammar `resource!` gives it. Its `;`
/// is optional, so reading up to the next `;` could swallow the sections after it.
fn parse_one_line(section: &Ident, body: ParseStream) -> Result<TokenStream> {
    let mut tokens = TokenStream::new();
    let take = |tokens: &mut TokenStream| -> Result<()> {
        tokens.extend([body.parse::<TokenTree>()?]);
        Ok(())
    };
    match section.to_string().as_str() {
        "embedded" => {}
        "extensions" | "notifiers" | "timestamps" => {
            if body.peek(syn::token::Bracket) {
                take(&mut tokens)?;
            }
        }
        // `extend path! { ... }`
        "extend" => {
            while !body.is_empty() && !body.peek(syn::token::Brace) && !body.peek(Token![;]) {
                take(&mut tokens)?;
            }
            if body.peek(syn::token::Brace) {
                take(&mut tokens)?;
            }
        }
        "table" | "optimistic_lock" | "data_layer" => {
            if body.peek(Token![:]) {
                take(&mut tokens)?;
            }
            take(&mut tokens)?;
        }
        "store" => {
            if body.peek(Token![:]) {
                take(&mut tokens)?;
            }
            let ty: syn::Type = body.parse()?;
            tokens.extend(quote! { #ty });
        }
        // Unknown to us: stop at `;` or at the next section, unless that name is part of
        // a path such as `crate::actions::X`.
        _ => {
            let mut after_colon = false;
            while !body.is_empty() && !body.peek(Token![;]) {
                let next_is_section = body
                    .fork()
                    .parse::<Ident>()
                    .is_ok_and(|ident| SECTION_NAMES.iter().any(|name| ident == name));
                if next_is_section && !after_colon {
                    break;
                }
                let tree = body.parse::<TokenTree>()?;
                after_colon = matches!(&tree, TokenTree::Punct(punct) if punct.as_char() == ':');
                tokens.extend([tree]);
            }
        }
    }
    Ok(tokens)
}

impl ResourceTokens {
    /// Parses the item an attribute macro receives: `resource! { ... }` or the bare DSL.
    pub fn from_item(item: TokenStream) -> Result<Self> {
        match syn::parse2::<ItemMacro>(item.clone()) {
            Ok(item_macro) => {
                let path = &item_macro.mac.path;
                if path.segments.last().is_none_or(|segment| segment.ident != "resource") {
                    return Err(Error::new_spanned(path, "expected `resource! { ... }`"));
                }
                let mut resource = syn::parse2::<Self>(item_macro.mac.tokens)?;
                // Doc comments describe the resource, so they go to the struct; rustc
                // would ignore them on the macro call.
                let (mut docs, call_attrs): (Vec<_>, Vec<_>) = item_macro
                    .attrs
                    .into_iter()
                    .partition(|attr| attr.path().is_ident("doc"));
                docs.append(&mut resource.outer_attrs);
                resource.outer_attrs = docs;
                resource.call_attrs = call_attrs;
                Ok(resource)
            }
            Err(_) => syn::parse2::<Self>(item),
        }
    }

    pub fn section(&self, name: &str) -> Option<&Section> {
        self.sections.iter().find(|s| s.name == name)
    }

    pub fn section_mut(&mut self, name: &str) -> Option<&mut Section> {
        self.sections.iter_mut().find(|s| s.name == name)
    }

    /// Removes and returns a section, for blocks only the transformer understands.
    pub fn take_section(&mut self, name: &str) -> Option<Section> {
        let index = self.sections.iter().position(|s| s.name == name)?;
        Some(self.sections.remove(index))
    }

    /// Appends tokens to a braced section, creating it when missing.
    pub fn append_to_section(&mut self, name: &str, tokens: TokenStream) {
        match self.section_mut(name) {
            Some(section) => {
                let existing = &section.tokens;
                section.tokens = quote! { #existing #tokens };
            }
            None => self.sections.push(Section {
                name: format_ident!("{name}"),
                tokens,
                braced: true,
            }),
        }
    }

    /// Whether `attributes { ... }` declares `name: Type`.
    pub fn declares_attribute(&self, name: &Ident) -> bool {
        self.declared_type(name).is_some()
    }

    /// The type `attributes { ... }` declares for `name`, up to its `[...]` options.
    pub fn declared_type(&self, name: &Ident) -> Option<Vec<TokenTree>> {
        let section = self.section("attributes")?;
        let trees: Vec<TokenTree> = section.tokens.clone().into_iter().collect();
        // `name:` must start an attribute (after `;`, `,`, or `#[...]`), and its `:` must
        // stand alone, so `super::name::Type` or `= name::X` do not count.
        let starts_item = |index: usize| match index.checked_sub(1).map(|i| &trees[i]) {
            None => true,
            Some(TokenTree::Punct(punct)) => matches!(punct.as_char(), ';' | ','),
            Some(TokenTree::Group(group)) => group.delimiter() == proc_macro2::Delimiter::Bracket,
            Some(_) => false,
        };
        let start = (0..trees.len().saturating_sub(1)).find(|&index| {
            match (&trees[index], &trees[index + 1]) {
                (TokenTree::Ident(ident), TokenTree::Punct(punct)) => {
                    ident == name
                        && punct.as_char() == ':'
                        && punct.spacing() == proc_macro2::Spacing::Alone
                        && starts_item(index)
                }
                _ => false,
            }
        })?;
        Some(
            trees[start + 2..]
                .iter()
                .take_while(|tree| match tree {
                    TokenTree::Punct(punct) => punct.as_char() != ';',
                    TokenTree::Group(group) => {
                        group.delimiter() != proc_macro2::Delimiter::Bracket
                    }
                    _ => true,
                })
                .cloned()
                .collect(),
        )
    }

    /// Adds an expression to `extensions`, in either its `[ ... ]` or `{ ... }` form.
    pub fn add_extension(&mut self, expr: TokenStream) -> Result<()> {
        match self.section_mut("extensions") {
            Some(section) if section.braced => {
                let existing = &section.tokens;
                section.tokens = quote! { #expr; #existing };
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
                section.tokens = quote! { #expr, #inner };
                section.braced = true;
            }
            None => self.append_to_section("extensions", quote! { #expr; }),
        }
        Ok(())
    }

    /// The actions in the `actions` section, or none when it is missing.
    pub fn actions(&self) -> Result<Vec<Action>> {
        match self.section("actions") {
            Some(section) => parse_actions(section.tokens.clone()),
            None => Ok(Vec::new()),
        }
    }

    /// Replaces the `actions` section.
    pub fn set_actions(&mut self, actions: &[Action]) {
        let tokens = render_actions(actions);
        match self.section_mut("actions") {
            Some(section) => {
                section.tokens = tokens;
                section.braced = true;
            }
            None => self.append_to_section("actions", tokens),
        }
    }

    /// The rebuilt `::ash_core::resource! { ... }` call.
    pub fn to_resource_macro(&self) -> TokenStream {
        let call_attrs = &self.call_attrs;
        let outer_attrs = &self.outer_attrs;
        let embedded = self.embedded.then(|| quote! { embedded });
        let name = &self.name;
        let sections = self.sections.iter().map(|section| {
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
        quote! {
            #(#call_attrs)*
            ::ash_core::resource! {
                #(#outer_attrs)*
                #embedded #name {
                    #(#sections)*
                }
            }
        }
    }
}

/// One action from the `actions` section, split so items can be appended to its body.
pub struct Action {
    pub attrs: Vec<Attribute>,
    pub kind: Ident,
    pub name: Ident,
    /// The `Type` in `generic name, Type { ... }`.
    pub returns: Option<TokenStream>,
    pub body: TokenStream,
}

impl Action {
    pub fn new(kind: &str, name: Ident, body: TokenStream) -> Self {
        Self {
            attrs: Vec::new(),
            kind: format_ident!("{kind}"),
            name,
            returns: None,
            body,
        }
    }

    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }

    pub fn append(&mut self, tokens: TokenStream) {
        let body = &self.body;
        self.body = quote! { #body #tokens };
    }
}

impl Parse for Action {
    fn parse(input: ParseStream) -> Result<Self> {
        let attrs = input.call(Attribute::parse_outer)?;
        let kind: Ident = input.parse()?;
        let name: Ident = input.parse()?;
        let mut returns = None;
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            let mut ty = TokenStream::new();
            while !input.is_empty() && !input.peek(syn::token::Brace) && !input.peek(Token![;]) {
                ty.extend([input.parse::<TokenTree>()?]);
            }
            returns = Some(ty);
        }
        let body = if input.peek(syn::token::Brace) {
            let content;
            syn::braced!(content in input);
            content.parse()?
        } else {
            TokenStream::new()
        };
        Ok(Self {
            attrs,
            kind,
            name,
            returns,
            body,
        })
    }
}

pub fn parse_actions(tokens: TokenStream) -> Result<Vec<Action>> {
    (|input: ParseStream| {
        let mut actions = Vec::new();
        loop {
            // `resource!` accepts `,` or `;` between actions.
            while input.peek(Token![,]) || input.peek(Token![;]) {
                let _ = input.parse::<TokenTree>()?;
            }
            if input.is_empty() {
                return Ok(actions);
            }
            actions.push(input.parse::<Action>()?);
        }
    })
    .parse2(tokens)
}

pub fn render_actions(actions: &[Action]) -> TokenStream {
    let actions = actions.iter().map(|action| {
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
    quote! { #(#actions)* }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn resource(tokens: TokenStream) -> ResourceTokens {
        ResourceTokens::from_item(tokens).unwrap()
    }

    #[test]
    fn round_trips_sections_and_embedded() {
        let parsed = resource(quote! {
            resource! {
                embedded Address {
                    table "addresses";
                    attributes { street: String; }
                    extensions [&FOO];
                }
            }
        });
        assert!(parsed.embedded);
        assert_eq!(parsed.name, "Address");
        assert_eq!(parsed.sections.len(), 3);
        let out = parsed.to_resource_macro().to_string();
        assert!(out.contains("embedded Address"), "{out}");
        assert!(out.contains("table \"addresses\" ;"), "{out}");
    }

    #[test]
    fn actions_keep_docs_returns_and_short_forms() {
        let actions = parse_actions(quote! {
            /// Reads things.
            read read { primary; }
            read all;
            generic total, i64 { run |_| async { Ok(1) }; }
        })
        .unwrap();
        let names: Vec<String> = actions.iter().map(|a| a.name.to_string()).collect();
        assert_eq!(names, ["read", "all", "total"]);
        assert_eq!(actions[0].attrs.len(), 1);
        assert!(actions[1].body.is_empty());
        assert_eq!(actions[2].returns.as_ref().unwrap().to_string(), "i64");
        let out = render_actions(&actions).to_string();
        assert!(out.contains("read all { }"), "{out}");
        assert!(out.contains("generic total , i64"), "{out}");
    }

    #[test]
    fn declares_attribute_needs_a_declaration() {
        let parsed = resource(quote! {
            Ticket { attributes { status_note: Option<String>; } }
        });
        assert!(!parsed.declares_attribute(&format_ident!("status")));
        assert!(parsed.declares_attribute(&format_ident!("status_note")));

        // A module named like the attribute, in a type or a default, is not a declaration.
        let parsed = resource(quote! {
            Ticket {
                attributes {
                    note: super::status::Note;
                    kind: Option<status::Kind> = status::DEFAULT;
                    /// The state.
                    #[allow(dead_code)]
                    state: String;
                }
            }
        });
        assert!(!parsed.declares_attribute(&format_ident!("status")));
        let ty = parsed.declared_type(&format_ident!("state")).unwrap();
        assert_eq!(quote! { #(#ty)* }.to_string(), "String");
    }

    #[test]
    fn one_line_sections_do_not_need_semicolons() {
        let parsed = resource(quote! {
            Note {
                table "notes"
                store crate::stores::Main
                extend tag! { label: "x"; }
                timestamps
                extensions [&A]
                attributes { id: Uuid [pk]; }
                actions { read read { primary; } }
            }
        });
        let names: Vec<String> = parsed.sections.iter().map(|s| s.name.to_string()).collect();
        assert_eq!(
            names,
            ["table", "store", "extend", "timestamps", "extensions", "attributes", "actions"]
        );
        assert_eq!(parsed.section("store").unwrap().tokens.to_string(), "crate :: stores :: Main");
        assert_eq!(
            parsed.section("extend").unwrap().tokens.to_string(),
            "tag ! { label : \"x\" ; }"
        );
    }

    #[test]
    fn attributes_stay_where_they_were_written() {
        let parsed = resource(quote! {
            /// Docs on the call.
            #[state_machine]
            resource! {
                #[derive(Default)]
                Note { attributes { id: Uuid [pk]; } }
            }
        });
        let out = parsed.to_resource_macro().to_string();
        let call = out.find(":: ash_core :: resource !").unwrap();
        assert!(out.find("# [state_machine]").unwrap() < call, "{out}");
        assert!(out.find("# [derive (Default)]").unwrap() > call, "{out}");
        assert!(out.find("Docs on the call.").unwrap() > call, "{out}");
    }

    #[test]
    fn rejects_repeated_sections_and_other_macros() {
        let err = ResourceTokens::from_item(quote! {
            Note { actions { read read; } actions { destroy destroy; } }
        })
        .err()
        .unwrap();
        assert!(err.to_string().contains("`actions` appears twice"), "{err}");

        let err = ResourceTokens::from_item(quote! { other! { Note {} } }).err().unwrap();
        assert!(err.to_string().contains("expected `resource! { ... }`"), "{err}");
    }

    #[test]
    fn actions_may_be_separated_by_commas_and_semicolons() {
        let actions = parse_actions(quote! {
            create create { primary; },
            read read;;
            destroy destroy { primary; };
        })
        .unwrap();
        let names: Vec<String> = actions.iter().map(|a| a.name.to_string()).collect();
        assert_eq!(names, ["create", "read", "destroy"]);
    }

    #[test]
    fn add_extension_handles_both_forms() {
        for extensions in [quote! { extensions [&A, &B]; }, quote! { extensions { &A; &B; } }] {
            let mut parsed = resource(quote! { Thing { #extensions } });
            parsed.add_extension(quote! { &C }).unwrap();
            let section = parsed.section("extensions").unwrap();
            assert!(section.braced);
            let tokens = section.tokens.to_string();
            assert!(tokens.starts_with("& C"), "{tokens}");
            assert!(tokens.contains("& A") && tokens.contains("& B"), "{tokens}");
        }
        let mut parsed = resource(quote! { Thing { table "things"; } });
        parsed.add_extension(quote! { &C }).unwrap();
        assert!(parsed.section("extensions").is_some());
    }
}
