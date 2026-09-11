use syn::parse::ParseStream;
use syn::parse::discouraged::Speculative;
use syn::punctuated::Punctuated;
use syn::{Error, Expr, Ident, Result, Token, Type};

use crate::ast_helpers::option_inner;
use crate::define::ast::{
    ActionKind, ActionSpec, ArgumentSpec, ChangeSpec, FieldAccept, PreparationSpec, ValidationSpec,
};

use super::helpers::{expr_to_ident, parse_i64, require_semi};

macro_rules! parse_braced {
    ($input:expr, $content:ident) => {
        let $content;
        let _ = syn::braced!($content in $input);
    };
}

pub fn parse_actions(input: ParseStream, errors: &mut Vec<Error>) -> Vec<ActionSpec> {
    let mut actions = Vec::new();

    while !input.is_empty() {
        if input.peek(Token![;]) || input.peek(Token![,]) {
            let _ = input.parse::<proc_macro2::TokenTree>();
            continue;
        }
        let fork = input.fork();
        match parse_one_action(&fork, errors) {
            Ok(act) => {
                input.advance_to(&fork);
                actions.push(act);
            }
            Err(e) => {
                errors.push(e);
                input.advance_to(&fork);
                if !super::recover::at_action_kind(input) {
                    super::recover::skip_action(input);
                }
            }
        }
    }

    actions
}

fn parse_one_action(input: ParseStream, errors: &mut Vec<Error>) -> Result<ActionSpec> {
    let outer_attrs = input.call(syn::Attribute::parse_outer)?;
    let kind_ident: Ident = input.parse()?;
    const ACTION_KINDS: &[&str] = &["create", "read", "update", "destroy", "generic"];
    let kind = match kind_ident.to_string().as_str() {
        "create" => ActionKind::Create,
        "read" => ActionKind::Read,
        "update" => ActionKind::Update,
        "destroy" => ActionKind::Destroy,
        "generic" => ActionKind::Generic,
        "action" => {
            errors.push(Error::new_spanned(
                &kind_ident,
                "use `generic`, not `action`",
            ));
            ActionKind::Generic
        }
        _ => {
            return Err(crate::ast_helpers::unknown_ident_error(
                &kind_ident,
                ACTION_KINDS,
                "action kind",
            ));
        }
    };

    let name: Ident = input.parse()?;
    let mut returns = None;
    if input.peek(Token![,]) {
        let _: Token![,] = input.parse()?;
        let ret_ty: Type = input.parse()?;
        returns = Some(ret_ty);
    }

    let mut primary = false;
    let mut accept = Vec::new();
    let mut arguments = Vec::new();
    let mut changes = Vec::new();
    let mut validations = Vec::new();
    let mut preparations = Vec::new();
    let mut persist_manual = false;
    let mut run_expr = None;
    let mut accept_kw = None;
    let mut change_kw = None;
    let mut validate_kw = None;
    let mut prepare_kw = None;
    let mut persist_kw = None;
    let mut returns_kw = None;
    let mut run_kw = None;
    let mut accept_span = None;

    if input.peek(Token![;]) {
        let _: Token![;] = input.parse()?;
    } else if input.peek(syn::token::Brace) {
        parse_braced!(input, body);
        while !body.is_empty() {
            let item_attrs = body.call(syn::Attribute::parse_outer)?;
            let item_ident: Ident = body.parse()?;
            match item_ident.to_string().as_str() {
                "primary" => {
                    primary = true;
                    if body.peek(Token![:]) || body.peek(Token![=]) {
                        errors.push(Error::new_spanned(
                            &item_ident,
                            "use `primary;`, not `primary true`",
                        ));
                        let _ = body.parse::<proc_macro2::TokenTree>()?;
                    }
                    if body.peek(syn::LitBool) {
                        let lit: syn::LitBool = body.parse()?;
                        errors.push(Error::new_spanned(
                            &lit,
                            "use `primary;`, not `primary true`",
                        ));
                        primary = lit.value;
                    }
                    require_semi(&body, errors, "`primary`");
                }
                "argument" => {
                    let a_name: Ident = body.parse()?;
                    let _: Token![:] = body.parse()?;
                    let a_ty: Type = body.parse()?;
                    let allow_nil = option_inner(&a_ty).is_some();
                    arguments.push(ArgumentSpec {
                        outer_attrs: item_attrs,
                        name: a_name,
                        ty: a_ty,
                        allow_nil,
                    });
                    require_semi(&body, errors, "argument");
                }
                "arguments" => {
                    errors.push(Error::new_spanned(
                        &item_ident,
                        "use `argument <name>: <type>;`, not `arguments { ... }`",
                    ));
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    parse_braced!(body, args_input);
                    while !args_input.is_empty() {
                        let outer_attrs = args_input.call(syn::Attribute::parse_outer)?;
                        let a_name: Ident = args_input.parse()?;
                        let _: Token![:] = args_input.parse()?;
                        let a_ty: Type = args_input.parse()?;
                        let allow_nil = option_inner(&a_ty).is_some();
                        arguments.push(ArgumentSpec {
                            outer_attrs,
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
                    if accept_kw.is_none() {
                        accept_kw = Some(item_ident.clone());
                    }
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    if body.peek(syn::token::Bracket) {
                        let items;
                        let _ = syn::bracketed!(items in body);
                        accept_span = Some(items.span());
                        let list = Punctuated::<Ident, Token![,]>::parse_terminated(&items)?;
                        for item in list {
                            accept.push(FieldAccept {
                                name: item,
                                ty: syn::parse_quote!(::ash_core::Value),
                                inferred: true,
                            });
                        }
                    } else if body.peek(syn::token::Brace) {
                        if kind != ActionKind::Generic {
                            errors.push(Error::new_spanned(
                                    &item_ident,
                                    "use `accept [field, ...];` on create/read/update/destroy — types come from attributes. Typed `accept { name: Type }` is only for generic actions",
                                ));
                        }
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
                                inferred: kind != ActionKind::Generic,
                            });
                        }
                    }
                    require_semi(&body, errors, "accept");
                }
                "change" => {
                    if change_kw.is_none() {
                        change_kw = Some(item_ident.clone());
                    }
                    let expr: Expr = body.parse()?;
                    changes.push(parse_change(&expr, errors)?);
                    require_semi(&body, errors, "change");
                }
                "before_action" => {
                    let expr: Expr = body.parse()?;
                    changes.push(ChangeSpec::BeforeAction(expr));
                    require_semi(&body, errors, "before_action");
                }
                "after_action" => {
                    let expr: Expr = body.parse()?;
                    changes.push(ChangeSpec::AfterAction(expr));
                    require_semi(&body, errors, "after_action");
                }
                "after_transaction" => {
                    let expr: Expr = body.parse()?;
                    changes.push(ChangeSpec::AfterTransaction(expr));
                    require_semi(&body, errors, "after_transaction");
                }
                "changes" => {
                    if change_kw.is_none() {
                        change_kw = Some(item_ident.clone());
                    }
                    errors.push(Error::new_spanned(
                        &item_ident,
                        "use `change <action>;`, not `changes [...]`",
                    ));
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    let items;
                    let _ = syn::bracketed!(items in body);
                    let exprs = Punctuated::<Expr, Token![,]>::parse_terminated(&items)?;
                    for expr in exprs {
                        changes.push(parse_change(&expr, errors)?);
                    }
                    if body.peek(Token![;]) {
                        let _: Token![;] = body.parse()?;
                    }
                }
                "validate" | "validation" => {
                    if validate_kw.is_none() {
                        validate_kw = Some(item_ident.clone());
                    }
                    if item_ident == "validation" {
                        errors.push(Error::new_spanned(
                            &item_ident,
                            "use `validate`, not `validation`",
                        ));
                    }
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    if body.peek(syn::token::Bracket) {
                        let items;
                        let _ = syn::bracketed!(items in body);
                        while !items.is_empty() {
                            validations.push(parse_validation(&items, errors)?);
                            if items.peek(Token![,]) {
                                let _: Token![,] = items.parse()?;
                            }
                        }
                    } else {
                        validations.push(parse_validation(&body, errors)?);
                    }
                    require_semi(&body, errors, "validate");
                }
                "validations" => {
                    if validate_kw.is_none() {
                        validate_kw = Some(item_ident.clone());
                    }
                    errors.push(Error::new_spanned(
                        &item_ident,
                        "use `validate <rule>;`, not `validations [...]`",
                    ));
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    let items;
                    let _ = syn::bracketed!(items in body);
                    while !items.is_empty() {
                        validations.push(parse_validation(&items, errors)?);
                        if items.peek(Token![,]) {
                            let _: Token![,] = items.parse()?;
                        }
                    }
                    if body.peek(Token![;]) {
                        let _: Token![;] = body.parse()?;
                    }
                }
                "prepare" | "preparation" => {
                    if prepare_kw.is_none() {
                        prepare_kw = Some(item_ident.clone());
                    }
                    if item_ident == "preparation" {
                        errors.push(Error::new_spanned(
                            &item_ident,
                            "use `prepare`, not `preparation`",
                        ));
                    }
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
                    require_semi(&body, errors, "prepare");
                }
                "preparations" => {
                    if prepare_kw.is_none() {
                        prepare_kw = Some(item_ident.clone());
                    }
                    errors.push(Error::new_spanned(
                        &item_ident,
                        "use `prepare <item>;`, not `preparations [...]`",
                    ));
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
                    if persist_kw.is_none() {
                        persist_kw = Some(item_ident.clone());
                    }
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    let mode: Ident = body.parse()?;
                    if mode == "manual" {
                        persist_manual = true;
                    } else {
                        return Err(Error::new_spanned(mode, "expected `manual`"));
                    }
                    require_semi(&body, errors, "persist");
                }
                "returns" => {
                    if returns_kw.is_none() {
                        returns_kw = Some(item_ident.clone());
                    }
                    if body.peek(Token![:]) {
                        let _: Token![:] = body.parse()?;
                    }
                    let ret_ty: Type = body.parse()?;
                    returns = Some(ret_ty);
                    require_semi(&body, errors, "returns");
                }
                "run" => {
                    if run_kw.is_none() {
                        run_kw = Some(item_ident.clone());
                    }
                    let expr: Expr = body.parse()?;
                    run_expr = Some(expr);
                    require_semi(&body, errors, "run");
                }
                _ => {
                    const ACTION_ITEM_NAMES: &[&str] = &[
                        "primary",
                        "argument",
                        "arguments",
                        "accept",
                        "change",
                        "changes",
                        "before_action",
                        "after_action",
                        "after_transaction",
                        "validate",
                        "validation",
                        "validations",
                        "prepare",
                        "preparation",
                        "preparations",
                        "persist",
                        "returns",
                        "run",
                    ];
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &item_ident,
                        ACTION_ITEM_NAMES,
                        "action item",
                    ));
                }
            }
        }
    }

    super::helpers::optional_semi(input)?;

    Ok(ActionSpec {
        outer_attrs,
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
        run_expr,
        accept_kw,
        change_kw,
        validate_kw,
        prepare_kw,
        persist_kw,
        returns_kw,
        run_kw,
        accept_span,
    })
}

fn arg_call_ident(expr: &Expr) -> Option<Ident> {
    let Expr::Call(call) = expr else {
        return None;
    };
    let Expr::Path(func) = &*call.func else {
        return None;
    };
    if func.path.get_ident()?.to_string() != "arg" {
        return None;
    }
    expr_to_ident(call.args.first()?).ok()
}

pub fn parse_change(expr: &Expr, errors: &mut Vec<Error>) -> Result<ChangeSpec> {
    let Expr::Call(call) = expr else {
        return Err(Error::new_spanned(
            expr,
            "expected change call like `set(field = value)` or `relate_actor(field)`",
        ));
    };

    let Expr::Path(func) = &*call.func else {
        return Err(Error::new_spanned(&call.func, "expected function name"));
    };
    let func_ident = func.path.get_ident();
    let func_name = func_ident
        .ok_or_else(|| Error::new_spanned(func, "expected identifier"))?
        .to_string();

    match func_name.as_str() {
        "set_attribute" => {
            errors.push(Error::new_spanned(
                func,
                "use `set(...)`, not `set_attribute(...)`",
            ));
            parse_set_change(call, false)
        }
        "set" => parse_set_change(call, false),
        "set_new_attribute" => {
            errors.push(Error::new_spanned(
                func,
                "use `set_new(...)`, not `set_new_attribute(...)`",
            ));
            parse_set_change(call, true)
        }
        "set_new" => parse_set_change(call, true),
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
        "manage_relationship" => {
            if call.args.len() == 1 {
                let relationship = expr_to_ident(&call.args[0])?;
                return Ok(ChangeSpec::ManageRelationship {
                    relationship,
                    rel_type: syn::Ident::new("direct_control", proc_macro2::Span::call_site()),
                });
            } else if call.args.len() == 2 {
                let relationship = expr_to_ident(&call.args[0])?;
                let rel_type = match &call.args[1] {
                    Expr::Assign(assign) => expr_to_ident(&assign.right)?,
                    other => expr_to_ident(other)?,
                };
                return Ok(ChangeSpec::ManageRelationship {
                    relationship,
                    rel_type,
                });
            }
            Err(Error::new_spanned(
                call,
                "expected `manage_relationship(rel)` or `manage_relationship(rel, type: create)`",
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
        "before_action" => {
            if call.args.len() == 1 {
                let expr = call.args[0].clone();
                return Ok(ChangeSpec::BeforeAction(expr));
            }
            Err(Error::new_spanned(call, "expected `before_action(expr)`"))
        }
        "after_action" => {
            if call.args.len() == 1 {
                let expr = call.args[0].clone();
                return Ok(ChangeSpec::AfterAction(expr));
            }
            Err(Error::new_spanned(call, "expected `after_action(expr)`"))
        }
        "after_transaction" => {
            if call.args.len() == 1 {
                let expr = call.args[0].clone();
                return Ok(ChangeSpec::AfterTransaction(expr));
            }
            Err(Error::new_spanned(
                call,
                "expected `after_transaction(expr)`",
            ))
        }
        _ => {
            const CHANGE_NAMES: &[&str] = &[
                "set",
                "set_new",
                "relate_actor",
                "set_from_arg",
                "manage_relationship",
                "before_action",
                "after_action",
                "after_transaction",
                "custom",
                "func",
            ];
            if let Some(ident) = func.path.get_ident() {
                Err(crate::ast_helpers::unknown_ident_error(
                    ident,
                    CHANGE_NAMES,
                    "change",
                ))
            } else {
                Err(Error::new_spanned(
                    func,
                    "unknown change, expected `set`, `set_new`, `relate_actor`, `set_from_arg`, `before_action`, `after_action`, `after_transaction`, `custom`, or `func`",
                ))
            }
        }
    }
}

fn parse_set_change(call: &syn::ExprCall, new: bool) -> Result<ChangeSpec> {
    if call.args.len() == 1 {
        if let Some(Expr::Assign(assign)) = call.args.first() {
            let field = expr_to_ident(&assign.left)?;
            if !new && let Some(argument) = arg_call_ident(&assign.right) {
                return Ok(ChangeSpec::SetFromArg { field, argument });
            }
            if new {
                return Ok(ChangeSpec::SetNew {
                    field,
                    value: (*assign.right).clone(),
                });
            }
            return Ok(ChangeSpec::Set {
                field,
                value: (*assign.right).clone(),
            });
        }
    } else if call.args.len() == 2 {
        let field = expr_to_ident(&call.args[0])?;
        if !new && let Some(argument) = arg_call_ident(&call.args[1]) {
            return Ok(ChangeSpec::SetFromArg { field, argument });
        }
        if new {
            return Ok(ChangeSpec::SetNew {
                field,
                value: call.args[1].clone(),
            });
        }
        return Ok(ChangeSpec::Set {
            field,
            value: call.args[1].clone(),
        });
    }
    let expected = if new {
        "expected `set_new(field = value)` or `set_new(field, value)`"
    } else {
        "expected `set(field = value)` or `set(field, value)`"
    };
    Err(Error::new_spanned(call, expected))
}

fn parse_named_usize(content: ParseStream, errors: &mut Vec<Error>) -> Result<(Ident, usize)> {
    if content.peek(syn::LitInt) {
        let lit: syn::LitInt = content.parse()?;
        errors.push(Error::new_spanned(
            &lit,
            "use `min: N` / `max: N`, not positional integers",
        ));
        return Ok((Ident::new("min", lit.span()), lit.base10_parse()?));
    }
    let key: Ident = content.parse()?;
    if content.peek(Token![=]) {
        errors.push(Error::new_spanned(
            &key,
            format!("use `{key}: N`, not `{key} = N`"),
        ));
        let _: Token![=] = content.parse()?;
    } else if content.peek(Token![:]) {
        let _: Token![:] = content.parse()?;
    } else {
        return Err(Error::new_spanned(
            &key,
            format!("expected `:` after `{key}`"),
        ));
    }
    let lit: syn::LitInt = content.parse()?;
    Ok((key, lit.base10_parse()?))
}

fn parse_named_i64(content: ParseStream, errors: &mut Vec<Error>) -> Result<(Ident, i64)> {
    if content.peek(syn::LitInt) || content.peek(Token![-]) {
        let span_ident = Ident::new("min", content.span());
        errors.push(Error::new(
            content.span(),
            "use `min: N` / `max: N`, not positional integers",
        ));
        return Ok((span_ident, parse_i64(content)?));
    }
    let key: Ident = content.parse()?;
    if content.peek(Token![=]) {
        errors.push(Error::new_spanned(
            &key,
            format!("use `{key}: N`, not `{key} = N`"),
        ));
        let _: Token![=] = content.parse()?;
    } else if content.peek(Token![:]) {
        let _: Token![:] = content.parse()?;
    } else {
        return Err(Error::new_spanned(
            &key,
            format!("expected `:` after `{key}`"),
        ));
    }
    Ok((key, parse_i64(content)?))
}

pub fn parse_validation(input: ParseStream, errors: &mut Vec<Error>) -> Result<ValidationSpec> {
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
                let (key, val) = parse_named_usize(&content, errors)?;
                match key.to_string().as_str() {
                    "min" => min = Some(val),
                    "max" => max = Some(val),
                    other => {
                        return Err(Error::new_spanned(
                            key,
                            format!(
                                "unknown string_length option `{other}`, expected `min` or `max`"
                            ),
                        ));
                    }
                }
            }

            if min.is_none() && max.is_none() {
                return Err(Error::new_spanned(
                    &field,
                    format!(
                        "string_length validation for '{field}' must specify at least one of 'min' or 'max'"
                    ),
                ));
            }

            if let (Some(min_v), Some(max_v)) = (min, max)
                && min_v > max_v
            {
                return Err(Error::new_spanned(
                    &field,
                    format!(
                        "invalid string_length for `{field}`: min ({min_v}) cannot be greater than max ({max_v})"
                    ),
                ));
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

            if allowed.is_empty() {
                return Err(Error::new_spanned(
                    &field,
                    format!(
                        "one_of validation for '{field}' must specify at least one allowed value"
                    ),
                ));
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
                let (key, val) = parse_named_i64(&content, errors)?;
                match key.to_string().as_str() {
                    "min" => min = Some(val),
                    "max" => max = Some(val),
                    other => {
                        return Err(Error::new_spanned(
                            key,
                            format!(
                                "unknown numericality option `{other}`, expected `min` or `max`"
                            ),
                        ));
                    }
                }
            }

            if min.is_none() && max.is_none() {
                return Err(Error::new_spanned(
                    &field,
                    format!(
                        "numericality validation for '{field}' must specify at least one of 'min' or 'max'"
                    ),
                ));
            }

            if let (Some(min_v), Some(max_v)) = (min, max)
                && min_v > max_v
            {
                return Err(Error::new_spanned(
                    &field,
                    format!(
                        "invalid numericality for `{field}`: min ({min_v}) cannot be greater than max ({max_v})"
                    ),
                ));
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
        _ => {
            const VALIDATION_NAMES: &[&str] = &[
                "present",
                "string_length",
                "one_of",
                "numericality",
                "custom",
                "func",
            ];
            Err(crate::ast_helpers::unknown_ident_error(
                &func_name,
                VALIDATION_NAMES,
                "validation",
            ))
        }
    }
}

pub fn parse_preparation(input: ParseStream) -> Result<PreparationSpec> {
    let func_name: Ident = input.parse()?;
    if input.peek(syn::token::Paren) {
        let content;
        syn::parenthesized!(content in input);
        match func_name.to_string().as_str() {
            "filter" => {
                let expr: Expr = content.parse()?;
                Ok(PreparationSpec::Filter { expr })
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
                Ok(PreparationSpec::Sort { field, descending })
            }
            "limit" => {
                let lit: syn::LitInt = content.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(PreparationSpec::Limit(val))
            }
            "offset" => {
                let lit: syn::LitInt = content.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(PreparationSpec::Offset(val))
            }
            _ => {
                const PREPARATION_NAMES: &[&str] = &["filter", "sort", "limit", "offset"];
                Err(crate::ast_helpers::unknown_ident_error(
                    &func_name,
                    PREPARATION_NAMES,
                    "preparation",
                ))
            }
        }
    } else if input.peek(Token![:]) || input.peek(Token![=]) {
        let _ = input.parse::<proc_macro2::TokenTree>()?;
        match func_name.to_string().as_str() {
            "filter" => {
                let expr: Expr = input.parse()?;
                Ok(PreparationSpec::Filter { expr })
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
                Ok(PreparationSpec::Sort { field, descending })
            }
            "limit" => {
                let lit: syn::LitInt = input.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(PreparationSpec::Limit(val))
            }
            "offset" => {
                let lit: syn::LitInt = input.parse()?;
                let val: usize = lit.base10_parse()?;
                Ok(PreparationSpec::Offset(val))
            }
            _ => {
                const PREPARATION_NAMES: &[&str] = &["filter", "sort", "limit", "offset"];
                Err(crate::ast_helpers::unknown_ident_error(
                    &func_name,
                    PREPARATION_NAMES,
                    "preparation",
                ))
            }
        }
    } else {
        Err(Error::new_spanned(
            func_name,
            "expected `(...)` or `: ...` after preparation name",
        ))
    }
}
