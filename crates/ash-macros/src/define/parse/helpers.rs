use syn::parse::ParseStream;
use syn::{Error, Expr, ExprPath, Ident, Lit, Result, Token};

pub fn expr_to_ident(expr: &Expr) -> Result<Ident> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path
            .get_ident()
            .cloned()
            .ok_or_else(|| Error::new_spanned(expr, "expected identifier")),
        other => Err(Error::new_spanned(other, "expected identifier")),
    }
}

pub fn ident_from_string(name: &str, span: proc_macro2::Span) -> Result<Ident> {
    let parsed: Ident = syn::parse_str(name)
        .map_err(|_| Error::new(span, format!("`{name}` is not a valid identifier")))?;
    Ok(Ident::new(&parsed.to_string(), span))
}

pub fn expr_to_field_ident(expr: &Expr) -> Result<Ident> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path
            .get_ident()
            .cloned()
            .ok_or_else(|| Error::new_spanned(expr, "expected field identifier")),
        Expr::Lit(syn::ExprLit {
            lit: Lit::Str(s), ..
        }) => ident_from_string(&s.value(), s.span()),
        other => Err(Error::new_spanned(other, "expected field identifier")),
    }
}

pub fn expr_to_lit(expr: &Expr) -> Result<Lit> {
    match expr {
        Expr::Lit(syn::ExprLit { lit, .. }) => Ok(lit.clone()),
        other => Err(Error::new_spanned(other, "expected literal value")),
    }
}

pub fn optional_semi(input: ParseStream) -> Result<()> {
    if input.peek(Token![;]) {
        let _: Token![;] = input.parse()?;
    }
    Ok(())
}

pub fn parse_i64(input: ParseStream) -> Result<i64> {
    let is_negative = if input.peek(Token![-]) {
        let _: Token![-] = input.parse()?;
        true
    } else {
        false
    };
    let lit: syn::LitInt = input.parse()?;
    let val: i64 = lit.base10_parse()?;
    Ok(if is_negative { -val } else { val })
}
