use syn::parse::ParseStream;
use syn::{Error, Expr, Ident, Result, Token, Type};

use crate::define::ast::{ArgumentSpec, CalculationExprSpec, CalculationSpec};

pub fn parse_calculations(input: ParseStream) -> Result<Vec<CalculationSpec>> {
    let mut calcs = Vec::new();
    while !input.is_empty() {
        let outer_attrs = input.call(syn::Attribute::parse_outer)?;
        let ident: Ident = input.parse()?;
        let mut arguments = Vec::new();
        if input.peek(syn::token::Paren) {
            let args_content;
            syn::parenthesized!(args_content in input);
            while !args_content.is_empty() {
                let arg_ident: Ident = args_content.parse()?;
                let _: Token![:] = args_content.parse()?;
                let arg_ty: Type = args_content.parse()?;
                arguments.push(ArgumentSpec {
                    name: arg_ident,
                    ty: arg_ty,
                    allow_nil: false,
                });
                if args_content.peek(Token![,]) {
                    let _: Token![,] = args_content.parse()?;
                }
            }
        }
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
                CalculationExprSpec::StringLength(stripped.trim().to_string())
            } else {
                CalculationExprSpec::LitString(val)
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
            arguments,
            ty,
            expr,
        });
    }
    Ok(calcs)
}

pub fn parse_calc_expr(expr: &syn::Expr) -> Result<CalculationExprSpec> {
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
                "arg" => {
                    let first = call
                        .args
                        .first()
                        .ok_or_else(|| Error::new_spanned(call, "expected argument name"))?;
                    match first {
                        syn::Expr::Path(p) => {
                            let f = p
                                .path
                                .get_ident()
                                .ok_or_else(|| Error::new_spanned(p, "expected identifier"))?;
                            Ok(CalculationExprSpec::Arg(f.to_string()))
                        }
                        syn::Expr::Lit(syn::ExprLit {
                            lit: syn::Lit::Str(s),
                            ..
                        }) => Ok(CalculationExprSpec::Arg(s.value())),
                        other => Err(Error::new_spanned(other, "expected argument identifier or string literal")),
                    }
                }
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
