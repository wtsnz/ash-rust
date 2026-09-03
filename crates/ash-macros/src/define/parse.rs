use super::ast::*;
use crate::ast_helpers::{last_ident, option_inner, vec_inner};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{
    BinOp, Error, Expr, ExprBinary, ExprCall, ExprClosure, ExprParen, ExprPath, Ident, Lit, Result,
    Token, Type,
};

macro_rules! parse_braced {
    ($input:expr, $content:ident) => {
        let $content;
        let _ = syn::braced!($content in $input);
    };
}

impl Parse for ResourceDefinition {
    fn parse(input: ParseStream) -> Result<Self> {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;

        let mut embedded = false;
        let mut header_kw: Ident = input.parse()?;
        if header_kw == "embedded" {
            embedded = true;
            if input.peek(Token![;]) {
                let _: Token![;] = input.parse()?;
            }
            header_kw = input.parse()?;
        }
        if header_kw != "resource" && header_kw != "name" {
            return Err(Error::new_spanned(
                header_kw,
                "expected `resource` or `name`",
            ));
        }
        let resource: Ident = input.parse()?;
        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        let mut table = None;
        let mut attributes = Vec::new();
        let mut relationships = Vec::new();
        let mut calculations = Vec::new();
        let mut aggregates = Vec::new();
        let mut actions = Vec::new();
        let mut policies = Vec::new();
        let mut field_policies = Vec::new();
        let mut extensions = Vec::new();
        let mut notifiers = Vec::new();
        let mut extends = Vec::new();
        let mut optimistic_lock = None;
        let mut identities = Vec::new();
        let mut data_layer = None;
        let mut timestamps = None;

        while !input.is_empty() {
            let section_ident: Ident = input.parse()?;
            if section_ident == "table" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let table_lit: syn::LitStr = input.parse()?;
                table = Some(table_lit.value());
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "attributes" {
                parse_braced!(input, content);
                let (parsed_attrs, attr_ts) = parse_attributes(&content)?;
                attributes = parsed_attrs;
                if timestamps.is_none() {
                    timestamps = attr_ts;
                }
            } else if section_ident == "relationships" {
                parse_braced!(input, content);
                relationships = parse_relationships(&content)?;
            } else if section_ident == "calculations" {
                parse_braced!(input, content);
                calculations = parse_calculations(&content)?;
            } else if section_ident == "aggregates" {
                parse_braced!(input, content);
                aggregates = parse_aggregates(&content)?;
            } else if section_ident == "actions" {
                parse_braced!(input, content);
                actions = parse_actions(&content)?;
            } else if section_ident == "policies" {
                parse_braced!(input, content);
                policies = parse_policies(&content)?;
            } else if section_ident == "field_policies" {
                parse_braced!(input, content);
                field_policies = parse_field_policies(&content)?;
            } else if section_ident == "extensions" {
                if input.peek(syn::token::Bracket) {
                    let items;
                    syn::bracketed!(items in input);
                    let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                    for item in list {
                        extensions.push(item);
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced!(input, content);
                    while !content.is_empty() {
                        let expr: Expr = content.parse()?;
                        extensions.push(expr);
                        if content.peek(Token![,]) || content.peek(Token![;]) {
                            let _ = content.parse::<proc_macro2::TokenTree>();
                        }
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "notifiers" {
                if input.peek(syn::token::Bracket) {
                    let items;
                    syn::bracketed!(items in input);
                    let list = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                    for item in list {
                        notifiers.push(item);
                    }
                } else if input.peek(syn::token::Brace) {
                    parse_braced!(input, content);
                    while !content.is_empty() {
                        let expr: Expr = content.parse()?;
                        notifiers.push(expr);
                        if content.peek(Token![,]) || content.peek(Token![;]) {
                            let _ = content.parse::<proc_macro2::TokenTree>();
                        }
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "extend" {
                let macro_path: syn::Path = input.parse()?;
                if input.peek(Token![!]) {
                    let _: Token![!] = input.parse()?;
                }
                let content;
                syn::braced!(content in input);
                let tokens: proc_macro2::TokenStream = content.parse()?;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                extends.push(crate::define::ast::ExtendSpec {
                    macro_path,
                    tokens,
                });
            } else if section_ident == "optimistic_lock" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let opt_ident: Ident = input.parse()?;
                optimistic_lock = Some(opt_ident);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "identities" {
                parse_braced!(input, content);
                identities = parse_identities(&content)?;
            } else if section_ident == "embedded" {
                embedded = true;
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "data_layer" {
                if input.peek(Token![:]) {
                    let _: Token![:] = input.parse()?;
                }
                let dl_ident: Ident = input.parse()?;
                data_layer = Some(dl_ident);
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
            } else if section_ident == "timestamps" {
                let mut created_at = syn::Ident::new("created_at", proc_macro2::Span::call_site());
                let mut updated_at = syn::Ident::new("updated_at", proc_macro2::Span::call_site());
                if input.peek(syn::token::Bracket) {
                    let names;
                    syn::bracketed!(names in input);
                    let list = Punctuated::<Ident, Token![,]>::parse_terminated(&names)?;
                    let vec: Vec<Ident> = list.into_iter().collect();
                    if vec.len() >= 2 {
                        created_at = vec[0].clone();
                        updated_at = vec[1].clone();
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                timestamps = Some(crate::define::ast::TimestampsSpec {
                    created_at,
                    updated_at,
                });
            } else {
                return Err(Error::new_spanned(
                    section_ident,
                    "expected `table`, `attributes`, `relationships`, `calculations`, `aggregates`, `actions`, `policies`, `field_policies`, `extensions`, `notifiers`, `extend`, `optimistic_lock`, `identities`, `embedded`, `data_layer`, or `timestamps`",
                ));
            }
        }

        if let Some(ref ts) = timestamps {
            if !attributes.iter().any(|a| a.ident == ts.created_at) {
                let c_ident = ts.created_at.clone();
                let str_ty: Type = syn::parse_str("String").unwrap();
                attributes.push(AttributeSpec {
                    outer_attrs: Vec::new(),
                    ident: c_ident,
                    ty: str_ty,
                    pk: false,
                    version: false,
                    generated: true,
                    atom: None,
                    is_enum: false,
                    default: None,
                    default_fn: None,
                });
            }
            if !attributes.iter().any(|a| a.ident == ts.updated_at) {
                let u_ident = ts.updated_at.clone();
                let str_ty: Type = syn::parse_str("String").unwrap();
                attributes.push(AttributeSpec {
                    outer_attrs: Vec::new(),
                    ident: u_ident,
                    ty: str_ty,
                    pk: false,
                    version: false,
                    generated: true,
                    atom: None,
                    is_enum: false,
                    default: None,
                    default_fn: None,
                });
            }
        }

        Ok(Self {
            outer_attrs,
            resource,
            table,
            attributes,
            relationships,
            calculations,
            aggregates,
            actions,
            policies,
            field_policies,
            extensions,
            notifiers,
            extends,
            optimistic_lock,
            identities,
            embedded,
            data_layer,
            timestamps,
        })
    }
}

fn parse_identities(input: ParseStream) -> Result<Vec<crate::define::ast::IdentitySpec>> {
    let mut identities = Vec::new();
    while !input.is_empty() {
        let ident_kw: Ident = input.parse()?;
        if ident_kw != "identity" {
            return Err(Error::new_spanned(ident_kw, "expected `identity`"));
        }
        let name: Ident = input.parse()?;
        if input.peek(Token![:]) {
            let _: Token![:] = input.parse()?;
        }
        let keys_content;
        syn::bracketed!(keys_content in input);
        let list = Punctuated::<Ident, Token![,]>::parse_terminated(&keys_content)?;
        let keys: Vec<Ident> = list.into_iter().collect();
        let mut message = None;
        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
            if input.peek(Ident) {
                let msg_kw: Ident = input.parse()?;
                if msg_kw == "message" {
                    if input.peek(Token![:]) || input.peek(Token![=]) {
                        let _ = input.parse::<proc_macro2::TokenTree>()?;
                    }
                    let msg_lit: syn::LitStr = input.parse()?;
                    message = Some(msg_lit.value());
                }
            }
        }
        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }
        identities.push(crate::define::ast::IdentitySpec {
            name,
            keys,
            message,
        });
    }
    Ok(identities)
}

fn parse_attributes(
    input: ParseStream,
) -> Result<(Vec<AttributeSpec>, Option<crate::define::ast::TimestampsSpec>)> {
    let mut attrs = Vec::new();
    let mut timestamps = None;
    while !input.is_empty() {
        if input.peek(Ident) {
            let fork = input.fork();
            let id: Ident = fork.parse()?;
            if id == "timestamps" {
                let _: Ident = input.parse()?;
                let mut created_at = syn::Ident::new("created_at", proc_macro2::Span::call_site());
                let mut updated_at = syn::Ident::new("updated_at", proc_macro2::Span::call_site());
                if input.peek(syn::token::Bracket) {
                    let names;
                    syn::bracketed!(names in input);
                    let list = Punctuated::<Ident, Token![,]>::parse_terminated(&names)?;
                    let vec: Vec<Ident> = list.into_iter().collect();
                    if vec.len() >= 2 {
                        created_at = vec[0].clone();
                        updated_at = vec[1].clone();
                    }
                }
                if input.peek(Token![;]) {
                    let _: Token![;] = input.parse()?;
                }
                timestamps = Some(crate::define::ast::TimestampsSpec {
                    created_at,
                    updated_at,
                });
                continue;
            }
        }

        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;

        let mut pk = false;
        let mut version = false;
        let mut generated = false;
        let mut default = None;
        let mut default_fn = None;
        let mut atom = None;
        let mut is_enum = false;

        if input.peek(Token![=]) {
            let _: Token![=] = input.parse()?;
            let expr: Expr = input.parse()?;
            default = Some(expr);
        }

        if input.peek(syn::token::Bracket) {
            let flags_content;
            syn::bracketed!(flags_content in input);
            while !flags_content.is_empty() {
                let flag_ident = flags_content.call(Ident::parse_any)?;
                if flag_ident == "pk" {
                    pk = true;
                    generated = true;
                } else if flag_ident == "version" {
                    version = true;
                } else if flag_ident == "generated" {
                    generated = true;
                } else if flag_ident == "enum" || flag_ident == "ash_enum" {
                    is_enum = true;
                } else if flag_ident == "default" {
                    if flags_content.peek(Token![:]) || flags_content.peek(Token![=]) {
                        let _ = flags_content.parse::<proc_macro2::TokenTree>()?;
                    }
                    let expr: Expr = flags_content.parse()?;
                    default = Some(expr);
                } else if flag_ident == "default_fn" {
                    if flags_content.peek(Token![:]) || flags_content.peek(Token![=]) {
                        let _ = flags_content.parse::<proc_macro2::TokenTree>()?;
                    }
                    let p: syn::Path = flags_content.parse()?;
                    default_fn = Some(p);
                } else if flag_ident == "atom" {
                    if flags_content.peek(Token![:]) {
                        let _: Token![:] = flags_content.parse()?;
                    } else if flags_content.peek(Token![=]) {
                        let _: Token![=] = flags_content.parse()?;
                    }
                    let s: syn::LitStr = flags_content.parse()?;
                    let list: Vec<String> = s
                        .value()
                        .split(',')
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect();
                    atom = Some(list);
                } else {
                    return Err(Error::new_spanned(
                        flag_ident,
                        "expected `pk`, `version`, `generated`, `default`, `default_fn`, `atom`, or `enum`",
                    ));
                }
                if flags_content.peek(Token![,]) {
                    let _: Token![,] = flags_content.parse()?;
                }
            }
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        attrs.push(AttributeSpec {
            outer_attrs,
            ident,
            ty,
            pk,
            version,
            generated,
            atom,
            is_enum,
            default,
            default_fn,
        });
    }
    Ok((attrs, timestamps))
}

fn parse_relationships(input: ParseStream) -> Result<Vec<RelationshipSpec>> {
    let mut rels = Vec::new();
    while !input.is_empty() {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let kind_ident: Ident = input.parse()?;
        let kind = match kind_ident.to_string().as_str() {
            "belongs_to" => RelType::BelongsTo,
            "has_many" => RelType::HasMany,
            "many_to_many" => RelType::ManyToMany,
            other => {
                return Err(Error::new_spanned(
                    kind_ident,
                    format!("unknown relationship `{other}`, expected `belongs_to`, `has_many`, or `many_to_many`"),
                ));
            }
        };

        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;

        let mut fk = None;
        let mut through = None;
        let mut source_attribute_on_join_resource = None;
        let mut destination_attribute_on_join_resource = None;
        let mut on_delete = OnDeleteSpec::Nothing;

        if input.peek(syn::token::Bracket) {
            let flags_content;
            syn::bracketed!(flags_content in input);
            while !flags_content.is_empty() {
                let flag_ident: Ident = flags_content.parse()?;
                match flag_ident.to_string().as_str() {
                    "fk" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let s: syn::LitStr = flags_content.parse()?;
                        fk = Some(s.value());
                    }
                    "through" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let th_ident: Ident = flags_content.parse()?;
                        through = Some(th_ident);
                    }
                    "source_fk" | "source_attribute_on_join_resource" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let s: syn::LitStr = flags_content.parse()?;
                        source_attribute_on_join_resource = Some(s.value());
                    }
                    "dest_fk" | "destination_attribute_on_join_resource" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let s: syn::LitStr = flags_content.parse()?;
                        destination_attribute_on_join_resource = Some(s.value());
                    }
                    "on_delete" => {
                        if flags_content.peek(Token![:]) {
                            let _: Token![:] = flags_content.parse()?;
                        } else if flags_content.peek(Token![=]) {
                            let _: Token![=] = flags_content.parse()?;
                        }
                        let val_str = if flags_content.peek(syn::LitStr) {
                            let s: syn::LitStr = flags_content.parse()?;
                            s.value()
                        } else {
                            let id: Ident = flags_content.parse()?;
                            id.to_string()
                        };
                        on_delete = match val_str.as_str() {
                            "cascade" => OnDeleteSpec::Cascade,
                            "nilify" => OnDeleteSpec::Nilify,
                            "restrict" => OnDeleteSpec::Restrict,
                            "nothing" => OnDeleteSpec::Nothing,
                            other => {
                                return Err(Error::new_spanned(
                                    flag_ident,
                                    format!("unknown on_delete value `{other}`, expected `cascade`, `nilify`, `restrict`, or `nothing`"),
                                ));
                            }
                        };
                    }
                    other => {
                        return Err(Error::new_spanned(
                            flag_ident,
                            format!("unknown relationship option `{other}`, expected `fk`, `through`, `source_fk`, `dest_fk`, or `on_delete`"),
                        ));
                    }
                }
                if flags_content.peek(Token![,]) {
                    let _: Token![,] = flags_content.parse()?;
                }
            }
        }

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        let (dest, struct_field_ty) = match kind {
            RelType::BelongsTo => {
                if let Some(inner) = option_inner(&ty) {
                    let dest_ident = last_ident(inner)
                        .ok_or_else(|| {
                            Error::new_spanned(inner, "expected destination type identifier")
                        })?
                        .clone();
                    (dest_ident, syn::parse_quote!(::ash_core::Rel<#ty>))
                } else {
                    let dest_ident = last_ident(&ty)
                        .ok_or_else(|| {
                            Error::new_spanned(&ty, "expected destination type identifier")
                        })?
                        .clone();
                    (
                        dest_ident.clone(),
                        syn::parse_quote!(::ash_core::Rel<::std::option::Option<#dest_ident>>),
                    )
                }
            }
            RelType::HasMany | RelType::ManyToMany => {
                if let Some(inner) = vec_inner(&ty) {
                    let dest_ident = last_ident(inner)
                        .ok_or_else(|| {
                            Error::new_spanned(inner, "expected destination type identifier")
                        })?
                        .clone();
                    (dest_ident, syn::parse_quote!(::ash_core::Rel<#ty>))
                } else {
                    let dest_ident = last_ident(&ty)
                        .ok_or_else(|| {
                            Error::new_spanned(&ty, "expected destination type identifier")
                        })?
                        .clone();
                    (
                        dest_ident.clone(),
                        syn::parse_quote!(::ash_core::Rel<::std::vec::Vec<#dest_ident>>),
                    )
                }
            }
        };

        rels.push(RelationshipSpec {
            outer_attrs,
            kind,
            ident,
            dest,
            struct_field_ty,
            fk,
            through,
            source_attribute_on_join_resource,
            destination_attribute_on_join_resource,
            on_delete,
        });
    }
    Ok(rels)
}

fn parse_calculations(input: ParseStream) -> Result<Vec<CalculationSpec>> {
    let mut calcs = Vec::new();
    while !input.is_empty() {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        let _: Token![=] = input.parse()?;

        let expr = if input.peek(syn::LitStr) {
            let lit: syn::LitStr = input.parse()?;
            let val = lit.value();
            if let Some(stripped) = val
                .strip_prefix("string_length(")
                .and_then(|s| s.strip_suffix(')'))
            {
                crate::define::ast::CalculationExprSpec::StringLength(stripped.trim().to_string())
            } else {
                crate::define::ast::CalculationExprSpec::LitString(val)
            }
        } else {
            let syn_expr: Expr = input.parse()?;
            parse_calc_expr(&syn_expr)?
        };

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        calcs.push(CalculationSpec {
            outer_attrs,
            ident,
            ty,
            expr,
        });
    }
    Ok(calcs)
}

fn parse_calc_expr(expr: &syn::Expr) -> Result<crate::define::ast::CalculationExprSpec> {
    use crate::define::ast::CalculationExprSpec;
    match expr {
        syn::Expr::Binary(b) => {
            let left = Box::new(parse_calc_expr(&b.left)?);
            let right = Box::new(parse_calc_expr(&b.right)?);
            match b.op {
                syn::BinOp::Add(_) => Ok(CalculationExprSpec::Add(left, right)),
                syn::BinOp::Sub(_) => Ok(CalculationExprSpec::Sub(left, right)),
                syn::BinOp::Mul(_) => Ok(CalculationExprSpec::Mul(left, right)),
                syn::BinOp::Div(_) => Ok(CalculationExprSpec::Div(left, right)),
                syn::BinOp::Eq(_) => Ok(CalculationExprSpec::Eq(left, right)),
                syn::BinOp::Ne(_) => Ok(CalculationExprSpec::Ne(left, right)),
                syn::BinOp::Gt(_) => Ok(CalculationExprSpec::Gt(left, right)),
                syn::BinOp::Ge(_) => Ok(CalculationExprSpec::Gte(left, right)),
                syn::BinOp::Lt(_) => Ok(CalculationExprSpec::Lt(left, right)),
                syn::BinOp::Le(_) => Ok(CalculationExprSpec::Lte(left, right)),
                _ => Err(Error::new_spanned(b, "unsupported binary operator in calculation")),
            }
        }
        syn::Expr::Call(call) => {
            let func_ident = match &*call.func {
                syn::Expr::Path(p) => p.path.get_ident().cloned(),
                _ => None,
            }
            .ok_or_else(|| Error::new_spanned(&call.func, "expected function name in calculation"))?;

            let name = func_ident.to_string();
            match name.as_str() {
                "string_length" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected argument"))?;
                    match first {
                        syn::Expr::Path(p) => {
                            let f = p
                                .path
                                .get_ident()
                                .ok_or_else(|| Error::new_spanned(p, "expected field"))?;
                            Ok(CalculationExprSpec::StringLength(f.to_string()))
                        }
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(s),
                            ..
                        }) => Ok(CalculationExprSpec::StringLength(s.value())),
                        other => {
                            let inner = parse_calc_expr(other)?;
                            Ok(CalculationExprSpec::Length(Box::new(inner)))
                        }
                    }
                }
                "length" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected argument"))?;
                    let inner = parse_calc_expr(first)?;
                    Ok(CalculationExprSpec::Length(Box::new(inner)))
                }
                "concat" => {
                    let mut args = Vec::new();
                    for a in &call.args {
                        args.push(parse_calc_expr(a)?);
                    }
                    Ok(CalculationExprSpec::Concat(args))
                }
                "coalesce" => {
                    let mut args = Vec::new();
                    for a in &call.args {
                        args.push(parse_calc_expr(a)?);
                    }
                    Ok(CalculationExprSpec::Coalesce(args))
                }
                "lower" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected argument"))?;
                    Ok(CalculationExprSpec::Lower(Box::new(parse_calc_expr(first)?)))
                }
                "upper" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected argument"))?;
                    Ok(CalculationExprSpec::Upper(Box::new(parse_calc_expr(first)?)))
                }
                "if_else" => {
                    if call.args.len() != 3 {
                        return Err(Error::new_spanned(
                            call,
                            "if_else expects 3 arguments: (condition, then_expr, else_expr)",
                        ));
                    }
                    let mut it = call.args.iter();
                    let cond = Box::new(parse_calc_expr(it.next().unwrap())?);
                    let then_expr = Box::new(parse_calc_expr(it.next().unwrap())?);
                    let else_expr = Box::new(parse_calc_expr(it.next().unwrap())?);
                    Ok(CalculationExprSpec::IfElse {
                        cond,
                        then_expr,
                        else_expr,
                    })
                }
                "custom" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected function path"))?;
                    let path = match first {
                        syn::Expr::Path(p) => p.path.clone(),
                        _ => return Err(Error::new_spanned(first, "expected function path")),
                    };
                    Ok(CalculationExprSpec::Custom(path))
                }
                other => Err(Error::new_spanned(
                    func_ident,
                    format!("unknown calculation function `{other}`"),
                )),
            }
        }
        syn::Expr::Path(p) if p.path.get_ident().is_some() => {
            let id = p.path.get_ident().unwrap().clone();
            if id == "null" {
                Ok(CalculationExprSpec::Null)
            } else {
                Ok(CalculationExprSpec::Field(id))
            }
        }
        syn::Expr::Lit(syn::ExprLit { lit, .. }) => match lit {
            syn::Lit::Int(n) => Ok(CalculationExprSpec::LitInt(n.base10_parse()?)),
            syn::Lit::Str(s) => Ok(CalculationExprSpec::LitString(s.value())),
            syn::Lit::Bool(b) => Ok(CalculationExprSpec::LitBool(b.value)),
            _ => Err(Error::new_spanned(lit, "unsupported literal in calculation")),
        },
        syn::Expr::Paren(p) => parse_calc_expr(&p.expr),
        other => Err(Error::new_spanned(other, "unsupported calculation expression")),
    }
}

fn parse_aggregates(input: ParseStream) -> Result<Vec<AggregateSpec>> {
    let mut aggs = Vec::new();
    while !input.is_empty() {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;
        let _: Token![:] = input.parse()?;
        let ty: Type = input.parse()?;
        let _: Token![=] = input.parse()?;

        let func: Ident = input.parse()?;
        let content;
        syn::parenthesized!(content in input);

        let func_str = func.to_string();
        let (kind, relationship, filter) = match func_str.as_str() {
            "count" => {
                let relationship: Ident = content.parse()?;
                let filter = parse_optional_aggregate_filter(&content)?;
                (AggregateKindSpec::Count, relationship, filter)
            }
            "exists" => {
                let relationship: Ident = content.parse()?;
                let filter = parse_optional_aggregate_filter(&content)?;
                (AggregateKindSpec::Exists, relationship, filter)
            }
            "first" => {
                let relationship: Ident = content.parse()?;
                let _: Token![,] = content.parse()?;
                let field: Ident = content.parse()?;
                let filter = parse_optional_aggregate_filter(&content)?;
                (AggregateKindSpec::First { field }, relationship, filter)
            }
            "sum" => {
                let relationship: Ident = content.parse()?;
                let _: Token![,] = content.parse()?;
                let field: Ident = content.parse()?;
                let filter = parse_optional_aggregate_filter(&content)?;
                (AggregateKindSpec::Sum { field }, relationship, filter)
            }
            other => {
                return Err(Error::new_spanned(
                    func,
                    format!("unknown aggregate function `{other}`, expected `count`, `exists`, `first`, or `sum`"),
                ));
            }
        };

        if input.peek(Token![,]) {
            let _: Token![,] = input.parse()?;
        } else if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        }

        aggs.push(AggregateSpec {
            outer_attrs,
            ident,
            ty,
            relationship,
            kind,
            filter,
        });
    }
    Ok(aggs)
}

fn parse_optional_aggregate_filter(content: ParseStream) -> Result<Option<AggregateFilterSpec>> {
    if content.is_empty() {
        return Ok(None);
    }
    if content.peek(Token![,]) {
        let _: Token![,] = content.parse()?;
    }
    if content.is_empty() {
        return Ok(None);
    }
    let filter_kw: Ident = content.parse()?;
    if filter_kw != "filter" {
        return Err(Error::new_spanned(filter_kw, "expected `filter:`"));
    }
    if content.peek(Token![:]) {
        let _: Token![:] = content.parse()?;
    } else if content.peek(Token![=]) {
        let _: Token![=] = content.parse()?;
    }
    let field: Ident = content.parse()?;
    let is_ne = if content.peek(Token![!=]) {
        let _: Token![!=] = content.parse()?;
        true
    } else if content.peek(Token![==]) {
        let _: Token![==] = content.parse()?;
        false
    } else if content.peek(Token![=]) {
        let _: Token![=] = content.parse()?;
        false
    } else {
        return Err(Error::new_spanned(field, "expected `==`, `!=`, or `=` in filter"));
    };
    let value: Lit = content.parse()?;
    if is_ne {
        Ok(Some(AggregateFilterSpec::Ne { field, value }))
    } else {
        Ok(Some(AggregateFilterSpec::Eq { field, value }))
    }
}

fn parse_actions(input: ParseStream) -> Result<Vec<ActionSpec>> {
    let mut actions = Vec::new();

    while !input.is_empty() {
        let kind_ident: Ident = input.parse()?;
        let kind = match kind_ident.to_string().as_str() {
            "create" => ActionKind::Create,
            "read" => ActionKind::Read,
            "update" => ActionKind::Update,
            "destroy" => ActionKind::Destroy,
            "generic" => ActionKind::Generic,
            _ => {
                return Err(Error::new_spanned(
                    kind_ident,
                    "expected action kind: create, read, update, destroy, or generic",
                ));
            }
        };

        let name: Ident = input.parse()?;

        let mut primary = false;
        let mut accept = Vec::new();
        let mut arguments = Vec::new();
        let mut changes = Vec::new();
        let mut validations = Vec::new();
        let mut preparations = Vec::new();
        let mut persist_manual = false;
        let mut returns = None;
        let mut run_closure = None;

        if input.peek(Token![;]) {
            let _: Token![;] = input.parse()?;
        } else if input.peek(syn::token::Brace) {
            parse_braced!(input, body);
            while !body.is_empty() {
                let item_ident: Ident = body.parse()?;
                match item_ident.to_string().as_str() {
                    "primary" => {
                        primary = true;
                        if body.peek(Token![:]) || body.peek(Token![=]) {
                            let _ = body.parse::<proc_macro2::TokenTree>()?;
                        }
                        if body.peek(syn::LitBool) {
                            let lit: syn::LitBool = body.parse()?;
                            primary = lit.value;
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "argument" => {
                        let a_name: Ident = body.parse()?;
                        let _: Token![:] = body.parse()?;
                        let a_ty: Type = body.parse()?;
                        let allow_nil = option_inner(&a_ty).is_some();
                        arguments.push(ArgumentSpec {
                            name: a_name,
                            ty: a_ty,
                            allow_nil,
                        });
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "arguments" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        parse_braced!(body, args_input);
                        while !args_input.is_empty() {
                            let a_name: Ident = args_input.parse()?;
                            let _: Token![:] = args_input.parse()?;
                            let a_ty: Type = args_input.parse()?;
                            let allow_nil = option_inner(&a_ty).is_some();
                            arguments.push(ArgumentSpec {
                                name: a_name,
                                ty: a_ty,
                                allow_nil,
                            });
                            if args_input.peek(Token![,]) {
                                let _: Token![,] = args_input.parse()?;
                            }
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "accept" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        if body.peek(syn::token::Bracket) {
                            let items;
                            let _ = syn::bracketed!(items in body);
                            let list = Punctuated::<Ident, Token![,]>::parse_terminated(&items)?;
                            for item in list {
                                accept.push(FieldAccept {
                                    name: item,
                                    ty: syn::parse_quote!(::ash_core::Value),
                                    inferred: true,
                                });
                            }
                        } else if body.peek(syn::token::Brace) {
                            parse_braced!(body, fields_input);
                            while !fields_input.is_empty() {
                                let f_name: Ident = fields_input.parse()?;
                                let _: Token![:] = fields_input.parse()?;
                                let f_ty: Type = fields_input.parse()?;
                                if fields_input.peek(Token![,]) {
                                    let _: Token![,] = fields_input.parse()?;
                                }
                                accept.push(FieldAccept {
                                    name: f_name,
                                    ty: f_ty,
                                    inferred: false,
                                });
                            }
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "change" => {
                        let expr: Expr = body.parse()?;
                        changes.push(parse_change(&expr)?);
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "changes" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        let items;
                        let _ = syn::bracketed!(items in body);
                        let exprs = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                        for expr in exprs {
                            changes.push(parse_change(&expr)?);
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "validate" | "validation" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        if body.peek(syn::token::Bracket) {
                            let items;
                            let _ = syn::bracketed!(items in body);
                            while !items.is_empty() {
                                validations.push(parse_validation(&items)?);
                                if items.peek(Token![,]) {
                                    let _: Token![,] = items.parse()?;
                                }
                            }
                        } else {
                            validations.push(parse_validation(&body)?);
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "validations" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        let items;
                        let _ = syn::bracketed!(items in body);
                        while !items.is_empty() {
                            validations.push(parse_validation(&items)?);
                            if items.peek(Token![,]) {
                                let _: Token![,] = items.parse()?;
                            }
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "prepare" | "preparation" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        if body.peek(syn::token::Bracket) {
                            let items;
                            let _ = syn::bracketed!(items in body);
                            while !items.is_empty() {
                                preparations.push(parse_preparation(&items)?);
                                if items.peek(Token![,]) {
                                    let _: Token![,] = items.parse()?;
                                }
                            }
                        } else {
                            preparations.push(parse_preparation(&body)?);
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "preparations" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        let items;
                        let _ = syn::bracketed!(items in body);
                        while !items.is_empty() {
                            preparations.push(parse_preparation(&items)?);
                            if items.peek(Token![,]) {
                                let _: Token![,] = items.parse()?;
                            }
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "persist" => {
                        if body.peek(Token![:]) {
                            let _: Token![:] = body.parse()?;
                        }
                        let mode: Ident = body.parse()?;
                        if mode == "manual" {
                            persist_manual = true;
                        } else {
                            return Err(Error::new_spanned(mode, "expected `manual`"));
                        }
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "returns" => {
                        let ret_ty: Type = body.parse()?;
                        returns = Some(ret_ty);
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    "run" => {
                        let closure: ExprClosure = body.parse()?;
                        run_closure = Some(closure);
                        if body.peek(Token![;]) {
                            let _: Token![;] = body.parse()?;
                        }
                    }
                    other => {
                        return Err(Error::new_spanned(
                            item_ident,
                            format!("unknown action item `{other}`"),
                        ));
                    }
                }
            }
        }

        actions.push(ActionSpec {
            kind,
            name,
            primary,
            accept,
            arguments,
            changes,
            validations,
            preparations,
            persist_manual,
            returns,
            run_closure,
        });
    }

    Ok(actions)
}

fn parse_change(expr: &Expr) -> Result<ChangeSpec> {
    let Expr::Call(call) = expr else {
        return Err(Error::new_spanned(
            expr,
            "expected change call like `set(field = value)` or `relate_actor(field)`",
        ));
    };

    let Expr::Path(func) = &*call.func else {
        return Err(Error::new_spanned(&call.func, "expected function name"));
    };
    let func_name = func
        .path
        .get_ident()
        .ok_or_else(|| Error::new_spanned(func, "expected identifier"))?
        .to_string();

    match func_name.as_str() {
        "set" | "set_attribute" => {
            if call.args.len() == 1 {
                if let Some(Expr::Assign(assign)) = call.args.first() {
                    let field = expr_to_ident(&assign.left)?;
                    let value = expr_to_lit(&assign.right)?;
                    return Ok(ChangeSpec::Set { field, value });
                }
            } else if call.args.len() == 2 {
                let field = expr_to_ident(&call.args[0])?;
                let value = expr_to_lit(&call.args[1])?;
                return Ok(ChangeSpec::Set { field, value });
            }
            Err(Error::new_spanned(
                call,
                "expected `set(field = value)` or `set(field, value)`",
            ))
        }
        "set_new" | "set_new_attribute" => {
            if call.args.len() == 1 {
                if let Some(Expr::Assign(assign)) = call.args.first() {
                    let field = expr_to_ident(&assign.left)?;
                    let value = expr_to_lit(&assign.right)?;
                    return Ok(ChangeSpec::SetNew { field, value });
                }
            } else if call.args.len() == 2 {
                let field = expr_to_ident(&call.args[0])?;
                let value = expr_to_lit(&call.args[1])?;
                return Ok(ChangeSpec::SetNew { field, value });
            }
            Err(Error::new_spanned(
                call,
                "expected `set_new(field = value)` or `set_new(field, value)`",
            ))
        }
        "relate_actor" => {
            if call.args.len() == 1 {
                let field = expr_to_ident(&call.args[0])?;
                return Ok(ChangeSpec::RelateActor { field });
            }
            Err(Error::new_spanned(call, "expected `relate_actor(field)`"))
        }
        "set_from_arg" | "set_from_argument" => {
            if call.args.len() == 2 {
                let field = expr_to_ident(&call.args[0])?;
                let argument = expr_to_ident(&call.args[1])?;
                return Ok(ChangeSpec::SetFromArg { field, argument });
            }
            Err(Error::new_spanned(
                call,
                "expected `set_from_arg(field, argument)`",
            ))
        }
        "custom" => {
            if call.args.len() == 1 {
                let expr = call.args[0].clone();
                return Ok(ChangeSpec::Custom(expr));
            }
            Err(Error::new_spanned(call, "expected `custom(expr)`"))
        }
        "func" => {
            if call.args.len() == 1 {
                let expr = call.args[0].clone();
                return Ok(ChangeSpec::Func(expr));
            }
            Err(Error::new_spanned(call, "expected `func(expr)`"))
        }
        other => Err(Error::new_spanned(
            func,
            format!("unknown change `{other}`, expected `set`, `set_new`, `relate_actor`, `set_from_arg`, `custom`, or `func`"),
        )),
    }
}

fn parse_i64(input: ParseStream) -> Result<i64> {
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

fn parse_validation(input: ParseStream) -> Result<ValidationSpec> {
    let func_name: Ident = input.parse()?;
    let content;
    syn::parenthesized!(content in input);

    match func_name.to_string().as_str() {
        "present" => {
            let field: Ident = content.parse()?;
            Ok(ValidationSpec::Present { field })
        }
        "string_length" => {
            let field: Ident = content.parse()?;
            let mut min = None;
            let mut max = None;

            while !content.is_empty() {
                let _: Token![,] = content.parse()?;
                if content.is_empty() {
                    break;
                }
                if content.peek(syn::LitInt) {
                    let lit: syn::LitInt = content.parse()?;
                    let val: usize = lit.base10_parse()?;
                    if min.is_none() {
                        min = Some(val);
                    } else if max.is_none() {
                        max = Some(val);
                    } else {
                        return Err(Error::new_spanned(lit, "unexpected extra argument"));
                    }
                } else {
                    let key: Ident = content.parse()?;
                    if content.peek(Token![=]) {
                        let _: Token![=] = content.parse()?;
                    } else if content.peek(Token![:]) {
                        let _: Token![:] = content.parse()?;
                    } else {
                        return Err(Error::new_spanned(key, "expected `=` or `:` after key"));
                    }
                    let lit: syn::LitInt = content.parse()?;
                    let val: usize = lit.base10_parse()?;
                    match key.to_string().as_str() {
                        "min" => min = Some(val),
                        "max" => max = Some(val),
                        other => {
                            return Err(Error::new_spanned(
                                key,
                                format!("unknown string_length option `{other}`, expected `min` or `max`"),
                            ));
                        }
                    }
                }
            }

            Ok(ValidationSpec::StringLength { field, min, max })
        }
        "one_of" => {
            let field: Ident = content.parse()?;
            let _: Token![,] = content.parse()?;
            let mut allowed = Vec::new();

            if content.peek(syn::token::Bracket) {
                let items;
                syn::bracketed!(items in content);
                while !items.is_empty() {
                    if items.peek(syn::LitStr) {
                        let lit: syn::LitStr = items.parse()?;
                        allowed.push(lit.value());
                    } else {
                        let id: Ident = items.parse()?;
                        allowed.push(id.to_string());
                    }
                    if items.peek(Token![,]) {
                        let _: Token![,] = items.parse()?;
                    }
                }
            } else {
                while !content.is_empty() {
                    if content.peek(syn::LitStr) {
                        let lit: syn::LitStr = content.parse()?;
                        allowed.push(lit.value());
                    } else {
                        let id: Ident = content.parse()?;
                        allowed.push(id.to_string());
                    }
                    if content.peek(Token![,]) {
                        let _: Token![,] = content.parse()?;
                    }
                }
            }

            Ok(ValidationSpec::OneOf { field, allowed })
        }
        "numericality" => {
            let field: Ident = content.parse()?;
            let mut min = None;
            let mut max = None;

            while !content.is_empty() {
                let _: Token![,] = content.parse()?;
                if content.is_empty() {
                    break;
                }
                if content.peek(syn::LitInt) || content.peek(Token![-]) {
                    let val = parse_i64(&content)?;
                    if min.is_none() {
                        min = Some(val);
                    } else if max.is_none() {
                        max = Some(val);
                    } else {
                        return Err(Error::new_spanned(field, "unexpected extra argument"));
                    }
                } else {
                    let key: Ident = content.parse()?;
                    if content.peek(Token![=]) {
                        let _: Token![=] = content.parse()?;
                    } else if content.peek(Token![:]) {
                        let _: Token![:] = content.parse()?;
                    } else {
                        return Err(Error::new_spanned(key, "expected `=` or `:` after key"));
                    }
                    let val = parse_i64(&content)?;
                    match key.to_string().as_str() {
                        "min" => min = Some(val),
                        "max" => max = Some(val),
                        other => {
                            return Err(Error::new_spanned(
                                key,
                                format!("unknown numericality option `{other}`, expected `min` or `max`"),
                            ));
                        }
                    }
                }
            }

            Ok(ValidationSpec::Numericality { field, min, max })
        }
        "custom" => {
            let expr: Expr = content.parse()?;
            Ok(ValidationSpec::Custom(expr))
        }
        "func" => {
            let expr: Expr = content.parse()?;
            Ok(ValidationSpec::Func(expr))
        }
        other => Err(Error::new_spanned(
            func_name,
            format!(
                "unknown validation `{other}`, expected `present`, `string_length`, `one_of`, `numericality`, `custom`, or `func`"
            ),
        )),
    }
}

fn parse_policy_effect(checks_block: ParseStream) -> Result<PolicyEffectSpec> {
    let auth_ident: Ident = checks_block.parse()?;
    let auth_str = auth_ident.to_string();
    let check_expr: Expr = checks_block.parse()?;
    let check = parse_check_expr(&check_expr)?;
    if checks_block.peek(Token![;]) {
        let _: Token![;] = checks_block.parse()?;
    }
    match auth_str.as_str() {
        "authorize_if" => Ok(PolicyEffectSpec::AuthorizeIf(check)),
        "authorize_unless" => Ok(PolicyEffectSpec::AuthorizeUnless(check)),
        "forbid_if" => Ok(PolicyEffectSpec::ForbidIf(check)),
        "forbid_unless" => Ok(PolicyEffectSpec::ForbidUnless(check)),
        _ => Err(Error::new_spanned(
            auth_ident,
            "expected `authorize_if`, `authorize_unless`, `forbid_if`, or `forbid_unless` statement",
        )),
    }
}

fn parse_policies(input: ParseStream) -> Result<Vec<PolicySpec>> {
    let mut policies = Vec::new();

    while !input.is_empty() {
        let policy_kw: Ident = input.parse()?;
        let (bypass, whens) = if policy_kw == "policy" {
            let whens = if input.peek(syn::token::Brace) {
                vec![PolicyWhenSpec::Always]
            } else {
                parse_whens(input)?
            };
            (false, whens)
        } else if policy_kw == "bypass" {
            let whens = if input.peek(syn::token::Brace) {
                vec![PolicyWhenSpec::Always]
            } else {
                parse_whens(input)?
            };
            (true, whens)
        } else {
            return Err(Error::new_spanned(policy_kw, "expected `policy` or `bypass`"));
        };

        parse_braced!(input, checks_block);
        let mut checks = Vec::new();

        while !checks_block.is_empty() {
            checks.push(parse_policy_effect(&checks_block)?);
        }

        policies.push(PolicySpec {
            bypass,
            whens,
            checks,
        });
    }

    Ok(policies)
}

fn parse_field_policies(input: ParseStream) -> Result<Vec<FieldPolicySpec>> {
    let mut fps = Vec::new();

    while !input.is_empty() {
        let kw: Ident = input.parse()?;
        if kw != "field" {
            return Err(Error::new_spanned(kw, "expected `field`"));
        }
        let field: Ident = input.parse()?;
        parse_braced!(input, checks_block);
        let mut checks = Vec::new();
        while !checks_block.is_empty() {
            checks.push(parse_policy_effect(&checks_block)?);
        }
        fps.push(FieldPolicySpec { field, checks });
    }

    Ok(fps)
}

fn parse_whens(input: ParseStream) -> Result<Vec<PolicyWhenSpec>> {
    let mut whens = Vec::new();

    while !input.peek(syn::token::Brace) && !input.is_empty() {
        let ident: Ident = input.parse()?;
        if ident == "always" {
            whens.push(PolicyWhenSpec::Always);
        } else if ident == "action" {
            let content;
            syn::parenthesized!(content in input);
            let action_ident: Ident = content.parse()?;
            whens.push(PolicyWhenSpec::ActionName(action_ident.to_string()));
        } else if ident == "action_type" {
            let content;
            syn::parenthesized!(content in input);
            let kind_ident: Ident = content.parse()?;
            let kind = match kind_ident.to_string().as_str() {
                "read" => ActionKind::Read,
                "create" => ActionKind::Create,
                "update" => ActionKind::Update,
                "destroy" => ActionKind::Destroy,
                "generic" => ActionKind::Generic,
                _ => {
                    return Err(Error::new_spanned(
                        kind_ident,
                        "unknown action type; expected read, create, update, destroy, or generic",
                    ));
                }
            };
            whens.push(PolicyWhenSpec::ActionKind(kind));
        } else {
            return Err(Error::new_spanned(
                ident,
                "expected `always`, `action(...)`, or `action_type(...)`",
            ));
        }

        if input.peek(Token![|]) {
            let _: Token![|] = input.parse()?;
        }
    }

    if whens.is_empty() {
        return Err(input.error("expected policy condition before `{`"));
    }

    Ok(whens)
}

fn parse_check_expr(expr: &Expr) -> Result<PolicyCheckExpr> {
    match expr {
        Expr::Path(ExprPath { path, .. }) if path.is_ident("always") => {
            Ok(PolicyCheckExpr::Always)
        }
        Expr::Path(ExprPath { path, .. }) if path.is_ident("actor_present") => {
            Ok(PolicyCheckExpr::ActorPresent)
        }
        Expr::Call(ExprCall { func, args, .. }) => {
            let Expr::Path(p) = &**func else {
                return Err(Error::new_spanned(func, "expected check function"));
            };
            let func_ident = p
                .path
                .get_ident()
                .ok_or_else(|| Error::new_spanned(p, "expected identifier"))?
                .to_string();

            match func_ident.as_str() {
                "relates_to" | "relates_to_actor" => {
                    let arg = args
                        .first()
                        .ok_or_else(|| Error::new_spanned(args, "expected field"))?;
                    let field = expr_to_field_name(arg)?;
                    Ok(PolicyCheckExpr::RelatesToActor(field))
                }
                "is_nil" => {
                    let arg = args
                        .first()
                        .ok_or_else(|| Error::new_spanned(args, "expected field"))?;
                    let field = expr_to_field_name(arg)?;
                    Ok(PolicyCheckExpr::IsNil(field))
                }
                "actor_eq" | "actor_attribute_equals" => {
                    if args.len() == 1 {
                        if let Some(Expr::Assign(assign)) = args.first() {
                            let attr = expr_to_field_name(&assign.left)?;
                            let value = expr_to_lit(&assign.right)?;
                            return Ok(PolicyCheckExpr::ActorAttributeEquals { attr, value });
                        }
                    } else if args.len() == 2 {
                        let attr = expr_to_field_name(&args[0])?;
                        let value = expr_to_lit(&args[1])?;
                        return Ok(PolicyCheckExpr::ActorAttributeEquals { attr, value });
                    }
                    Err(Error::new_spanned(
                        args,
                        "expected `actor_eq(attr = value)` or `actor_eq(attr, value)`",
                    ))
                }
                "eq" => {
                    if args.len() == 1 {
                        if let Some(Expr::Assign(assign)) = args.first() {
                            let field = expr_to_field_name(&assign.left)?;
                            let value = expr_to_lit(&assign.right)?;
                            return Ok(PolicyCheckExpr::Eq { field, value });
                        }
                    } else if args.len() == 2 {
                        let field = expr_to_field_name(&args[0])?;
                        let value = expr_to_lit(&args[1])?;
                        return Ok(PolicyCheckExpr::Eq { field, value });
                    }
                    Err(Error::new_spanned(
                        args,
                        "expected `eq(field = value)` or `eq(field, value)`",
                    ))
                }
                other => Err(Error::new_spanned(
                    func,
                    format!("unknown policy check `{other}`"),
                )),
            }
        }
        Expr::Binary(ExprBinary {
            op: BinOp::And(_),
            left,
            right,
            ..
        }) => {
            let mut parts = Vec::new();
            match parse_check_expr(left)? {
                PolicyCheckExpr::And(sub) => parts.extend(sub),
                other => parts.push(other),
            }
            match parse_check_expr(right)? {
                PolicyCheckExpr::And(sub) => parts.extend(sub),
                other => parts.push(other),
            }
            Ok(PolicyCheckExpr::And(parts))
        }
        Expr::Binary(ExprBinary {
            op: BinOp::Or(_),
            left,
            right,
            ..
        }) => {
            let mut parts = Vec::new();
            match parse_check_expr(left)? {
                PolicyCheckExpr::Or(sub) => parts.extend(sub),
                other => parts.push(other),
            }
            match parse_check_expr(right)? {
                PolicyCheckExpr::Or(sub) => parts.extend(sub),
                other => parts.push(other),
            }
            Ok(PolicyCheckExpr::Or(parts))
        }
        Expr::Paren(ExprParen { expr, .. }) => parse_check_expr(expr),
        other => Err(Error::new_spanned(
            other,
            "unsupported policy check; expected relates_to, is_nil, actor_eq, actor_present, always, &&, or ||",
        )),
    }
}

fn expr_to_ident(expr: &Expr) -> Result<Ident> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path
            .get_ident()
            .cloned()
            .ok_or_else(|| Error::new_spanned(expr, "expected identifier")),
        other => Err(Error::new_spanned(other, "expected identifier")),
    }
}

fn expr_to_field_name(expr: &Expr) -> Result<String> {
    match expr {
        Expr::Path(ExprPath { path, .. }) => path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .ok_or_else(|| Error::new_spanned(expr, "expected field name")),
        Expr::Lit(syn::ExprLit {
            lit: Lit::Str(s), ..
        }) => Ok(s.value()),
        other => Err(Error::new_spanned(other, "expected field identifier")),
    }
}

fn expr_to_lit(expr: &Expr) -> Result<Lit> {
    match expr {
        Expr::Lit(syn::ExprLit { lit, .. }) => Ok(lit.clone()),
        other => Err(Error::new_spanned(other, "expected literal value")),
    }
}

fn parse_preparation(input: ParseStream) -> Result<crate::define::ast::PreparationSpec> {
    let func_name: Ident = input.parse()?;
    if input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in input);
        match func_name.to_string().as_str() {
            "filter" => {
                let expr: Expr = content.parse()?;
                Ok(crate::define::ast::PreparationSpec::Filter { expr })
            }
            "sort" => {
                let field: Ident = content.parse()?;
                let mut descending = false;
                if content.peek(Token![,]) {
                    let _: Token![,] = content.parse()?;
                    if content.peek(Ident) {
                        let dir: Ident = content.parse()?;
                        if dir == "desc" || dir == "descending" {
                            descending = true;
                        } else if dir == "asc" || dir == "ascending" {
                            descending = false;
                        } else {
                            return Err(Error::new_spanned(dir, "expected `asc` or `desc`"));
                        }
                    }
                }
                Ok(crate::define::ast::PreparationSpec::Sort { field, descending })
            }
            "limit" => {
                let lit: syn::LitInt = content.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(crate::define::ast::PreparationSpec::Limit(val))
            }
            "offset" => {
                let lit: syn::LitInt = content.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(crate::define::ast::PreparationSpec::Offset(val))
            }
            other => Err(Error::new_spanned(
                func_name,
                format!("unknown preparation `{other}`, expected `filter`, `sort`, `limit`, or `offset`"),
            )),
        }
    } else if input.peek(Token![:]) || input.peek(Token![=]) {
        let _ = input.parse::<proc_macro2::TokenTree>()?;
        match func_name.to_string().as_str() {
            "filter" => {
                let expr: Expr = input.parse()?;
                Ok(crate::define::ast::PreparationSpec::Filter { expr })
            }
            "sort" => {
                let field: Ident = input.parse()?;
                let mut descending = false;
                if input.peek(Token![,]) {
                    let _: Token![,] = input.parse()?;
                    if input.peek(Ident) {
                        let dir: Ident = input.parse()?;
                        if dir == "desc" || dir == "descending" {
                            descending = true;
                        }
                    }
                }
                Ok(crate::define::ast::PreparationSpec::Sort { field, descending })
            }
            "limit" => {
                let lit: syn::LitInt = input.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(crate::define::ast::PreparationSpec::Limit(val))
            }
            "offset" => {
                let lit: syn::LitInt = input.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(crate::define::ast::PreparationSpec::Offset(val))
            }
            other => Err(Error::new_spanned(
                func_name,
                format!("unknown preparation `{other}`"),
            )),
        }
    } else {
        Err(Error::new_spanned(
            func_name,
            "expected `(...)` or `: ...` after preparation name",
        ))
    }
}
