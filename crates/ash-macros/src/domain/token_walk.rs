use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::{quote, quote_spanned};
use syn::Ident;

struct Walked {
    resources: Vec<Ident>,
    actions: Vec<(Ident, Ident)>,
}

fn is_resource_ident(id: &Ident) -> bool {
    let name = id.to_string();
    name.chars().next().is_some_and(|c| c.is_uppercase())
        && !matches!(
            name.as_str(),
            "String" | "Uuid" | "Option" | "Vec" | "Self" | "Some" | "None"
        )
}

fn is_action_ident(id: &Ident) -> bool {
    let name = id.to_string();
    !matches!(
        name.as_str(),
        "args" | "on" | "get_by" | "record" | "id" | "define" | "action" | "resources"
    ) && name
        .chars()
        .next()
        .is_some_and(|c| c == '_' || c.is_lowercase())
}

fn walk_domain(stream: TokenStream, out: &mut Walked) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Ident(id) = &tokens[i]
            && id == "resources"
            && let Some(TokenTree::Group(g)) = tokens.get(i + 1)
            && g.delimiter() == Delimiter::Brace
        {
            walk_resources(g.stream(), out);
            i += 2;
            continue;
        }
        if let TokenTree::Group(g) = &tokens[i] {
            walk_domain(g.stream(), out);
        }
        i += 1;
    }
}

fn walk_resources(stream: TokenStream, out: &mut Walked) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Ident(id) = &tokens[i] {
            if id == "resource" {
                i += 1;
                continue;
            }
            if is_resource_ident(id) {
                out.resources.push(id.clone());
                if let Some(TokenTree::Group(g)) = tokens.get(i + 1)
                    && g.delimiter() == Delimiter::Brace
                {
                    walk_resource_body(id, g.stream(), out);
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
}

fn walk_resource_body(resource: &Ident, stream: TokenStream, out: &mut Walked) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        if let TokenTree::Ident(id) = &tokens[i]
            && id == "action"
        {
            let mut j = i + 1;
            if let Some(TokenTree::Punct(p)) = tokens.get(j)
                && p.as_char() == ':'
            {
                j += 1;
            }
            if let Some(TokenTree::Ident(action)) = tokens.get(j)
                && is_action_ident(action)
            {
                out.actions.push((resource.clone(), action.clone()));
            }
        }
        if let TokenTree::Group(g) = &tokens[i] {
            walk_resource_body(resource, g.stream(), out);
        }
        i += 1;
    }
}

pub fn expand_probes(input: &TokenStream) -> TokenStream {
    let mut walked = Walked {
        resources: Vec::new(),
        actions: Vec::new(),
    };
    walk_domain(input.clone(), &mut walked);
    if walked.resources.is_empty() && walked.actions.is_empty() {
        return quote! {};
    }

    let resource_probes = walked.resources.iter().map(|id| {
        quote_spanned! { id.span() =>
            __ash_assert_resource::<#id>();
        }
    });
    let action_probes = walked.actions.iter().map(|(resource, action)| {
        quote_spanned! { action.span() =>
            let _ = <#resource>::#action::<::ash_memory::Memory>;
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
            #[cfg(rust_analyzer)]
            fn __ash_domain_token_walk() {
                #[allow(dead_code)]
                fn __ash_assert_resource<T: ::ash_core::Resource>() {}
                #(#resource_probes)*
                if false {
                    #(#action_probes)*
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    #[test]
    fn test_walk_resources_and_action_names() {
        let tokens = quote! {
            Helpdesk {
                resources {
                    Ticket {
                        define open_ticket action: open args: [subject: String];
                    };
                    Representative;
                }
            }
        };
        let out = expand_probes(&tokens).to_string();
        assert!(out.contains("Ticket"), "missing Ticket: {out}");
        assert!(
            out.contains("Representative"),
            "missing Representative: {out}"
        );
        assert!(out.contains("open"), "missing action open: {out}");
        assert!(
            out.contains("__ash_domain_token_walk"),
            "missing walk helper: {out}"
        );
    }
}
