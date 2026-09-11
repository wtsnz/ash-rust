use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::{quote, quote_spanned};
use syn::Ident;

struct Walked {
    fields: Vec<Ident>,
}

fn is_keyword(name: &str) -> bool {
    matches!(
        name,
        "accept"
            | "change"
            | "validate"
            | "prepare"
            | "policy"
            | "argument"
            | "primary"
            | "persist"
            | "manual"
            | "returns"
            | "run"
            | "filter"
            | "sort"
            | "limit"
            | "offset"
            | "present"
            | "string_length"
            | "one_of"
            | "numericality"
            | "set"
            | "set_new"
            | "set_from_arg"
            | "relate_actor"
            | "action"
            | "action_type"
            | "always"
            | "actor_eq"
            | "true"
            | "false"
            | "Self"
            | "Some"
            | "None"
            | "create"
            | "read"
            | "update"
            | "destroy"
            | "generic"
            | "on"
            | "record"
            | "min"
            | "max"
            | "desc"
            | "asc"
            | "eq"
            | "ne"
            | "lt"
            | "lte"
            | "gt"
            | "gte"
    )
}

fn is_field_ident(id: &Ident) -> bool {
    let name = id.to_string();
    if is_keyword(&name) {
        return false;
    }
    name.chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_lowercase())
}

fn collect_comma_idents(stream: TokenStream, out: &mut Vec<Ident>) {
    for tree in stream {
        match tree {
            TokenTree::Ident(id) if is_keyword(&id.to_string()) => break,
            TokenTree::Ident(id) if is_field_ident(&id) => out.push(id),
            TokenTree::Group(_) => {}
            _ => {}
        }
    }
}

fn first_field_ident(stream: TokenStream) -> Option<Ident> {
    for tree in stream {
        match tree {
            TokenTree::Ident(id) if is_field_ident(&id) => return Some(id),
            TokenTree::Ident(id) if is_keyword(&id.to_string()) => return None,
            _ => {}
        }
    }
    None
}

fn collect_snake_idents(stream: TokenStream, out: &mut Vec<Ident>) {
    for tree in stream {
        match tree {
            TokenTree::Ident(id) if is_field_ident(&id) => out.push(id),
            TokenTree::Group(g) => collect_snake_idents(g.stream(), out),
            _ => {}
        }
    }
}

fn walk(stream: TokenStream, out: &mut Walked) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Ident(id) = &tokens[i] {
            let name = id.to_string();
            let next = tokens.get(i + 1);
            if name == "accept" {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Bracket
                {
                    collect_comma_idents(g.stream(), &mut out.fields);
                    walk(g.stream(), out);
                    i += 2;
                    continue;
                }
                i += 1;
                while i < tokens.len() {
                    match &tokens[i] {
                        TokenTree::Ident(id) if is_keyword(&id.to_string()) => break,
                        TokenTree::Ident(id) if is_field_ident(id) => {
                            out.fields.push(id.clone());
                            i += 1;
                        }
                        TokenTree::Punct(p) if p.as_char() == ';' => break,
                        TokenTree::Group(_) => break,
                        _ => i += 1,
                    }
                }
                continue;
            } else if name == "action" {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    collect_comma_idents(g.stream(), &mut out.fields);
                    i += 2;
                    continue;
                }
            } else if matches!(name.as_str(), "count" | "exists" | "first" | "sum") {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    if let Some(field) = first_field_ident(g.stream()) {
                        out.fields.push(field);
                    }
                    i += 2;
                    continue;
                }
            } else if name == "set" || name == "set_new" {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    if let Some(field) = first_field_ident(g.stream()) {
                        out.fields.push(field);
                    }
                    i += 2;
                    continue;
                }
            } else if name == "set_from_arg" || name == "relate_actor" {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    if let Some(field) = first_field_ident(g.stream()) {
                        out.fields.push(field);
                    }
                    i += 2;
                    continue;
                }
            } else if name == "filter" {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    collect_snake_idents(g.stream(), &mut out.fields);
                    i += 2;
                    continue;
                }
            } else if matches!(
                name.as_str(),
                "present" | "string_length" | "one_of" | "numericality"
            ) {
                if let Some(TokenTree::Group(g)) = next
                    && g.delimiter() == Delimiter::Parenthesis
                {
                    if let Some(field) = first_field_ident(g.stream()) {
                        out.fields.push(field);
                    }
                    i += 2;
                    continue;
                }
            }
        }
        if let TokenTree::Group(g) = &tokens[i] {
            walk(g.stream(), out);
        }
        i += 1;
    }
}

pub fn expand_probes(input: &TokenStream, resource: Option<&Ident>) -> TokenStream {
    let Some(resource) = resource else {
        return quote! {};
    };
    let mut walked = Walked { fields: Vec::new() };
    walk(input.clone(), &mut walked);
    if walked.fields.is_empty() {
        return quote! {};
    }

    let field_probes = walked.fields.iter().map(|id| {
        quote_spanned! { id.span() =>
            let _ = &__ash_record.#id;
        }
    });

    quote! {
        #[doc(hidden)]
        const _: () = {
            #[allow(
                dead_code,
                unused_variables,
                unused_imports,
                unreachable_code,
                clippy::all,
                clippy::pedantic
            )]
            fn __ash_token_walk_probes(__ash_record: &#resource) {
                #(#field_probes)*
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proc_macro2::Group;
    use quote::quote;

    #[test]
    fn test_walk_accept_set_filter_and_action() {
        let tokens = quote! {
            Ticket {
                attributes {
                    id: Uuid [pk];
                    subject: String;
                    archived: bool;
                }
                actions {
                    create open {
                        accept [sub];
                        change set(statu = "open");
                    }
                    read read {
                        prepare filter(archived == false);
                    }
                }
                policies {
                    policy action(open) {
                        authorize_if always;
                    }
                }
            }
        };
        let resource = Ident::new("Ticket", proc_macro2::Span::call_site());
        let out = expand_probes(&tokens, Some(&resource)).to_string();
        assert!(out.contains("sub"), "missing accept ident: {out}");
        assert!(out.contains("statu"), "missing set lhs: {out}");
        assert!(out.contains("archived"), "missing filter field: {out}");
        assert!(out.contains("open"), "missing policy action: {out}");
        assert!(
            out.contains("__ash_token_walk_probes"),
            "missing walk helper: {out}"
        );
    }

    #[test]
    fn test_walk_accept_without_brackets_and_aggregates() {
        let tokens = quote! {
            Ticket {
                attributes {
                    id: Uuid [pk];
                    subject: String;
                }
                aggregates {
                    comment_count: Option<i64> = count(comments);
                }
                actions {
                    create open {
                        accept subject
                    }
                }
            }
        };
        let resource = Ident::new("Ticket", proc_macro2::Span::call_site());
        let out = expand_probes(&tokens, Some(&resource)).to_string();
        assert!(out.contains("subject"), "missing unbracketed accept: {out}");
        assert!(out.contains("comments"), "missing count rel: {out}");
    }

    #[test]
    fn test_walk_unclosed_accept_stops_at_next_keyword() {
        let mut accept_body = quote! { sub };
        accept_body.extend(quote! { change set(status = "open") });
        let group = Group::new(Delimiter::Bracket, accept_body);
        let tokens = quote! {
            Ticket {
                actions {
                    create open {
                        accept #group
                    }
                }
            }
        };
        let resource = Ident::new("Ticket", proc_macro2::Span::call_site());
        let out = expand_probes(&tokens, Some(&resource)).to_string();
        assert!(out.contains("sub"), "missing partial accept ident: {out}");
        assert!(out.contains("status"), "missing set field: {out}");
        assert!(
            !out.contains(". change") && !out.contains(".change"),
            "should not probe keyword `change`: {out}"
        );
    }
}
