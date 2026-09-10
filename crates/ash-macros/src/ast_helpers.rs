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

pub fn unknown_ident_error(typo: &Ident, candidates: &[&str], kind: &str) -> syn::Error {
    let name = typo.to_string();
    if let Some(suggestion) = find_closest_match(&name, candidates.iter().copied()) {
        syn::Error::new_spanned(
            typo,
            format!("unknown {kind} `{name}`. Did you mean `{suggestion}`?"),
        )
    } else {
        syn::Error::new_spanned(
            typo,
            format!(
                "unknown {kind} `{name}`. Expected one of: {}",
                candidates.join(", ")
            ),
        )
    }
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
}
