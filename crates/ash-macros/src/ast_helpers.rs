use syn::{
    Expr, ExprLit, GenericArgument, Ident, Lit, PathArguments, Result, Type, TypePath,
};

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
