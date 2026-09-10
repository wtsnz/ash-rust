use syn::parse::ParseStream;
use syn::{BinOp, Error, Expr, ExprBinary, ExprCall, ExprParen, ExprPath, Ident, Result, Token};

use crate::define::ast::{
    ActionKind, FieldPolicySpec, PolicyCheckExpr, PolicyEffectSpec, PolicySpec, PolicyWhenSpec,
};

use super::helpers::{expr_to_field_name, expr_to_lit};

macro_rules! parse_braced {
    ($input:expr, $content:ident) => {
        let $content;
        let _ = syn::braced!($content in $input);
    };
}

pub fn parse_policies(input: ParseStream) -> Result<Vec<PolicySpec>> {
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
            const POLICY_DECLS: &[&str] = &["policy", "bypass"];
            return Err(crate::ast_helpers::unknown_ident_error(
                &policy_kw,
                POLICY_DECLS,
                "policy declaration",
            ));
        };

        parse_braced!(input, checks_block);
        let mut checks = Vec::new();

        while !checks_block.is_empty() {
            checks.push(parse_policy_effect(&checks_block)?);
        }

        super::helpers::optional_semi(input)?;

        policies.push(PolicySpec {
            bypass,
            whens,
            checks,
        });
    }

    Ok(policies)
}

pub fn parse_field_policies(input: ParseStream) -> Result<Vec<FieldPolicySpec>> {
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
        super::helpers::optional_semi(input)?;
        fps.push(FieldPolicySpec { field, checks });
    }

    Ok(fps)
}

pub fn parse_policy_effect(checks_block: ParseStream) -> Result<PolicyEffectSpec> {
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
        _ => {
            const POLICY_EFFECTS: &[&str] = &[
                "authorize_if",
                "authorize_unless",
                "forbid_if",
                "forbid_unless",
            ];
            Err(crate::ast_helpers::unknown_ident_error(
                &auth_ident,
                POLICY_EFFECTS,
                "policy statement",
            ))
        }
    }
}

pub fn parse_whens(input: ParseStream) -> Result<Vec<PolicyWhenSpec>> {
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
                    const ACTION_TYPES: &[&str] =
                        &["read", "create", "update", "destroy", "generic"];
                    return Err(crate::ast_helpers::unknown_ident_error(
                        &kind_ident,
                        ACTION_TYPES,
                        "action type",
                    ));
                }
            };
            whens.push(PolicyWhenSpec::ActionKind(kind));
        } else {
            const WHENS: &[&str] = &["always", "action", "action_type"];
            return Err(crate::ast_helpers::unknown_ident_error(
                &ident,
                WHENS,
                "policy when condition",
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

pub fn parse_check_expr(expr: &Expr) -> Result<PolicyCheckExpr> {
    match expr {
        Expr::Path(ExprPath { path, .. }) if path.is_ident("always") => Ok(PolicyCheckExpr::Always),
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
                _ => {
                    const POLICY_CHECKS: &[&str] = &[
                        "relates_to",
                        "relates_to_actor",
                        "is_nil",
                        "actor_eq",
                        "actor_attribute_equals",
                        "eq",
                    ];
                    if let Some(ident) = p.path.get_ident() {
                        Err(crate::ast_helpers::unknown_ident_error(
                            ident,
                            POLICY_CHECKS,
                            "policy check",
                        ))
                    } else {
                        Err(Error::new_spanned(func, "unknown policy check"))
                    }
                }
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
