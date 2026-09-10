use proc_macro2::TokenStream;
use quote::quote;

use crate::define::ast::CalculationExprSpec;

pub fn calc_expr_to_tokens(expr: &CalculationExprSpec) -> TokenStream {
    match expr {
        CalculationExprSpec::Arg(a) => {
            let a = a.to_string();
            quote! { ::ash_core::Expr::Arg(#a) }
        }
        CalculationExprSpec::StringLength(s) => {
            let s = s.to_string();
            quote! { ::ash_core::Expr::StringLength(#s) }
        }
        CalculationExprSpec::Field(f) => {
            let f_str = f.to_string();
            quote! { ::ash_core::Expr::Field(#f_str) }
        }
        CalculationExprSpec::LitInt(n) => {
            quote! { ::ash_core::Expr::LitInt(#n) }
        }
        CalculationExprSpec::LitString(s) => {
            quote! { ::ash_core::Expr::LitString(#s) }
        }
        CalculationExprSpec::LitBool(b) => {
            quote! { ::ash_core::Expr::LitBool(#b) }
        }
        CalculationExprSpec::Null => {
            quote! { ::ash_core::Expr::Null }
        }
        CalculationExprSpec::Length(inner) => {
            let inner_tok = calc_expr_to_tokens(inner);
            quote! { ::ash_core::Expr::Length(&#inner_tok) }
        }
        CalculationExprSpec::Lower(inner) => {
            let inner_tok = calc_expr_to_tokens(inner);
            quote! { ::ash_core::Expr::Lower(&#inner_tok) }
        }
        CalculationExprSpec::Upper(inner) => {
            let inner_tok = calc_expr_to_tokens(inner);
            quote! { ::ash_core::Expr::Upper(&#inner_tok) }
        }
        CalculationExprSpec::Concat(parts) => {
            let part_toks: Vec<_> = parts.iter().map(calc_expr_to_tokens).collect();
            quote! { ::ash_core::Expr::Concat(&[#(&#part_toks),*]) }
        }
        CalculationExprSpec::Coalesce(parts) => {
            let part_toks: Vec<_> = parts.iter().map(calc_expr_to_tokens).collect();
            quote! { ::ash_core::Expr::Coalesce(&[#(&#part_toks),*]) }
        }
        CalculationExprSpec::Add(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Add(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Sub(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Sub(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Mul(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Mul(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Div(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Div(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Eq(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Eq(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Ne(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Ne(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Gt(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Gt(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Gte(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Gte(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Lt(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Lt(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::Lte(l, r) => {
            let l_tok = calc_expr_to_tokens(l);
            let r_tok = calc_expr_to_tokens(r);
            quote! { ::ash_core::Expr::Lte(&#l_tok, &#r_tok) }
        }
        CalculationExprSpec::IfElse {
            cond,
            then_expr,
            else_expr,
        } => {
            let c_tok = calc_expr_to_tokens(cond);
            let t_tok = calc_expr_to_tokens(then_expr);
            let e_tok = calc_expr_to_tokens(else_expr);
            quote! {
                ::ash_core::Expr::IfElse {
                    cond: &#c_tok,
                    then_expr: &#t_tok,
                    else_expr: &#e_tok,
                }
            }
        }
        CalculationExprSpec::Custom(path) => {
            quote! { ::ash_core::Expr::Custom(#path) }
        }
    }
}
