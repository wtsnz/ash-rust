use proc_macro2::Delimiter;
use syn::parse::ParseStream;
use syn::{Ident, Token};

pub const SECTION_NAMES: &[&str] = &[
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
    "multitenancy",
    "actor",
];

pub const ACTION_KINDS: &[&str] = &["create", "read", "update", "destroy", "generic"];

pub fn skip_one_tree(input: ParseStream) -> bool {
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

pub fn skip_group(input: ParseStream, delimiter: Delimiter) -> bool {
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

pub fn skip_to_semi(input: ParseStream) {
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

/// Skip the rest of a section after its keyword has been consumed.
pub fn skip_section_body(input: ParseStream) {
    if input.peek(Token![:]) || input.peek(Token![=]) {
        let _ = skip_one_tree(input);
    }
    if skip_group(input, Delimiter::Brace)
        || skip_group(input, Delimiter::Bracket)
        || skip_group(input, Delimiter::Parenthesis)
    {
        let _ = input.parse::<Token![;]>();
        return;
    }
    skip_to_semi(input);
}

/// Skip one list item (attribute, relationship, calculation, ...).
pub fn skip_item(input: ParseStream) {
    while !input.is_empty() {
        if input.peek(Token![,]) || input.peek(Token![;]) {
            let _ = skip_one_tree(input);
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

fn peek_ident_is(input: ParseStream, names: &[&str]) -> bool {
    if !input.peek(Ident) {
        return false;
    }
    let fork = input.fork();
    match fork.parse::<Ident>() {
        Ok(id) => names.iter().any(|n| id == n),
        Err(_) => false,
    }
}

pub fn at_action_kind(input: ParseStream) -> bool {
    peek_ident_is(input, ACTION_KINDS)
}

pub fn at_section_name(input: ParseStream) -> bool {
    peek_ident_is(input, SECTION_NAMES)
}

/// Skip a broken action. Stops before the next action kind.
pub fn skip_action(input: ParseStream) {
    while !input.is_empty() {
        if at_action_kind(input) {
            return;
        }
        if skip_group(input, Delimiter::Brace) {
            return;
        }
        if input.peek(Token![;]) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if !skip_one_tree(input) {
            return;
        }
    }
}

pub fn skip_until_section_or_end(input: ParseStream) {
    while !input.is_empty() {
        if at_section_name(input) {
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

/// Skip a broken `policy` / `bypass` item. Stops before the next one.
pub fn skip_policy(input: ParseStream) {
    while !input.is_empty() {
        if peek_ident_is(input, &["policy", "bypass"]) {
            return;
        }
        if skip_group(input, Delimiter::Brace) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if input.peek(Token![;]) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if !skip_one_tree(input) {
            return;
        }
    }
}

/// Skip a broken `field` policy. Stops before the next `field`.
pub fn skip_field_policy(input: ParseStream) {
    while !input.is_empty() {
        if peek_ident_is(input, &["field"]) {
            return;
        }
        if skip_group(input, Delimiter::Brace) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if input.peek(Token![;]) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if !skip_one_tree(input) {
            return;
        }
    }
}

/// Skip a broken `identity` item. Stops before the next `identity`.
pub fn skip_identity(input: ParseStream) {
    while !input.is_empty() {
        if peek_ident_is(input, &["identity"]) {
            return;
        }
        if skip_group(input, Delimiter::Brace)
            || skip_group(input, Delimiter::Bracket)
            || skip_group(input, Delimiter::Parenthesis)
        {
            continue;
        }
        if input.peek(Token![;]) {
            let _ = input.parse::<Token![;]>();
            return;
        }
        if !skip_one_tree(input) {
            return;
        }
    }
}
