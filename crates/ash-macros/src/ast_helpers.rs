use syn::{Expr, ExprLit, GenericArgument, Ident, Lit, PathArguments, Result, Type, TypePath};

pub fn screaming_snake(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.extend(c.to_uppercase());
    }
    out
}

pub fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i != 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

pub fn pascal_case(s: &str) -> String {
    let mut out = String::new();
    let mut capitalize = true;
    for c in s.chars() {
        if c == '_' {
            capitalize = true;
        } else if capitalize {
            out.extend(c.to_uppercase());
            capitalize = false;
        } else {
            out.push(c);
        }
    }
    out
}

pub fn type_path(ty: &Type) -> Option<&TypePath> {
    match ty {
        Type::Path(p) => Some(p),
        _ => None,
    }
}

pub fn last_ident(ty: &Type) -> Option<&Ident> {
    type_path(ty)?.path.segments.last().map(|s| &s.ident)
}

pub fn is_uuid(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|i| i == "Uuid")
}

pub fn is_string(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|i| i == "String")
}

pub fn is_i64(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|i| i == "i64")
}

pub fn is_bool(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|i| i == "bool")
}

pub fn is_integer(ty: &Type) -> bool {
    last_ident(ty).is_some_and(|i| {
        matches!(
            i.to_string().as_str(),
            "i8" | "i16"
                | "i32"
                | "i64"
                | "i128"
                | "isize"
                | "u8"
                | "u16"
                | "u32"
                | "u64"
                | "u128"
                | "usize"
        )
    })
}

pub fn generic_arg0(ty: &Type) -> Option<&Type> {
    let path = type_path(ty)?;
    let args = &path.path.segments.last()?.arguments;
    let PathArguments::AngleBracketed(args) = args else {
        return None;
    };
    match args.args.first()? {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    }
}

pub fn option_inner(ty: &Type) -> Option<&Type> {
    if last_ident(ty)? == "Option" {
        generic_arg0(ty)
    } else {
        None
    }
}

pub fn vec_inner(ty: &Type) -> Option<&Type> {
    if last_ident(ty)? == "Vec" {
        generic_arg0(ty)
    } else {
        None
    }
}

#[allow(dead_code)]
pub fn rel_inner(ty: &Type) -> syn::Result<&Type> {
    if last_ident(ty).is_some_and(|i| i == "Rel") {
        generic_arg0(ty).ok_or_else(|| syn::Error::new_spanned(ty, "Rel needs a type argument"))
    } else {
        Err(syn::Error::new_spanned(ty, "expected Rel<…>"))
    }
}

#[allow(dead_code)]
pub fn lit_string(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(syn::Error::new_spanned(other, "expected string literal")),
    }
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let m = a_chars.len();
    let n = b_chars.len();
    let mut dp = vec![vec![0; n + 1]; m + 1];
    for (i, row) in dp.iter_mut().enumerate().take(m + 1) {
        row[0] = i;
    }
    for (j, val) in dp[0].iter_mut().enumerate().take(n + 1) {
        *val = j;
    }
    for i in 1..=m {
        for j in 1..=n {
            let cost = if a_chars[i - 1] == b_chars[j - 1] {
                0
            } else {
                1
            };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[m][n]
}

pub fn find_closest_match<'a, I>(input: &str, candidates: I) -> Option<&'a str>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut best: Option<(&'a str, usize)> = None;
    let input_lower = input.to_lowercase();
    for cand in candidates {
        let cand_lower = cand.to_lowercase();
        let dist = levenshtein(&input_lower, &cand_lower);
        let max_dist = (input.len().max(cand.len()) / 2).max(2);
        if dist <= max_dist {
            match best {
                None => best = Some((cand, dist)),
                Some((_, prev_dist)) if dist < prev_dist => best = Some((cand, dist)),
                _ => {}
            }
        }
    }
    best.map(|(c, _)| c)
}

fn available_label(kind: &str) -> String {
    if kind.ends_with('s') {
        format!("Available {kind}")
    } else {
        format!("Available {kind}s")
    }
}

pub fn unknown_ident_error(typo: &Ident, candidates: &[&str], kind: &str) -> syn::Error {
    let name = typo.to_string();
    let available = format!("{}: {}", available_label(kind), candidates.join(", "));
    if let Some(suggestion) = find_closest_match(&name, candidates.iter().copied()) {
        syn::Error::new_spanned(
            typo,
            format!("unknown {kind} `{name}`. Did you mean `{suggestion}`? {available}"),
        )
    } else {
        syn::Error::new_spanned(typo, format!("unknown {kind} `{name}`. {available}"))
    }
}

pub fn extract_resource_ident(input: &proc_macro2::TokenStream) -> Option<Ident> {
    extract_ident_after_keyword(input, &["resource", "name"])
}

pub fn extract_domain_ident(input: &proc_macro2::TokenStream) -> Option<Ident> {
    extract_ident_after_keyword(input, &["domain", "name"])
}

fn extract_ident_after_keyword(
    input: &proc_macro2::TokenStream,
    keywords: &[&str],
) -> Option<Ident> {
    let tokens: Vec<proc_macro2::TokenTree> = input.clone().into_iter().collect();
    let mut i = 0;
    while i < tokens.len() {
        if let proc_macro2::TokenTree::Ident(id) = &tokens[i]
            && keywords.iter().any(|kw| id == kw)
            && let Some(proc_macro2::TokenTree::Ident(name)) = tokens.get(i + 1)
        {
            return Some(name.clone());
        }
        i += 1;
    }
    None
}

pub fn make_deprecated_warning(span: proc_macro2::Span, msg: &str) -> proc_macro2::TokenStream {
    quote::quote_spanned! { span =>
        const _: () = {
            #[deprecated(note = #msg)]
            #[allow(non_upper_case_globals)]
            const DEPRECATED_SYNTAX: () = ();
            let _ = DEPRECATED_SYNTAX;
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        assert_eq!(levenshtein("action", "actions"), 1);
        assert_eq!(levenshtein("validats", "validate"), 1);
        assert_eq!(levenshtein("presence", "present"), 2);
        assert_eq!(levenshtein("same", "same"), 0);
    }

    #[test]
    fn test_find_closest_match() {
        let sections = &[
            "table",
            "attributes",
            "relationships",
            "actions",
            "policies",
        ];
        assert_eq!(
            find_closest_match("action", sections.iter().copied()),
            Some("actions")
        );
        assert_eq!(
            find_closest_match("attribute", sections.iter().copied()),
            Some("attributes")
        );
        assert_eq!(
            find_closest_match("xyz12345", sections.iter().copied()),
            None
        );

        let validations = &["present", "string_length", "one_of", "numericality"];
        assert_eq!(
            find_closest_match("presence", validations.iter().copied()),
            Some("present")
        );
        assert_eq!(
            find_closest_match("str_length", validations.iter().copied()),
            Some("string_length")
        );

        let changes = &["set", "set_attribute", "relate_actor", "set_from_arg"];
        assert_eq!(
            find_closest_match("set_attr", changes.iter().copied()),
            Some("set_attribute")
        );
    }

    #[test]
    fn test_unknown_ident_error_includes_suggestion_and_candidates() {
        let typo: Ident = syn::parse_str("subjet").unwrap();
        let err = unknown_ident_error(
            &typo,
            &["id", "subject", "status", "opener_id"],
            "attribute",
        );
        let msg = err.to_string();
        assert!(msg.contains("unknown attribute `subjet`"), "got: {msg}");
        assert!(msg.contains("Did you mean `subject`?"), "got: {msg}");
        assert!(
            msg.contains("Available attributes: id, subject, status, opener_id"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_unknown_ident_error_lists_candidates_without_suggestion() {
        let typo: Ident = syn::parse_str("zzzzzzzz").unwrap();
        let err = unknown_ident_error(&typo, &["id", "subject", "status"], "attribute");
        let msg = err.to_string();
        assert!(msg.contains("unknown attribute `zzzzzzzz`"), "got: {msg}");
        assert!(!msg.contains("Did you mean"), "got: {msg}");
        assert!(
            msg.contains("Available attributes: id, subject, status"),
            "got: {msg}"
        );
    }

    #[test]
    fn test_extract_resource_ident_from_header() {
        let tokens = quote::quote! {
            /// docs
            resource Ticket;
            attributes { id: Uuid [pk] }
        };
        let ident = extract_resource_ident(&tokens).expect("resource ident");
        assert_eq!(ident.to_string(), "Ticket");
    }

    #[test]
    fn test_extract_domain_ident_from_header() {
        let tokens = quote::quote! {
            /// docs
            domain Helpdesk;
            resources { Ticket }
        };
        let ident = extract_domain_ident(&tokens).expect("domain ident");
        assert_eq!(ident.to_string(), "Helpdesk");
    }
}
