use crate::error::{Error, Result};
use crate::resource::{AttrType, ResourceDef};
use crate::value::{FieldMap, Value};

#[derive(Clone, Copy, Debug)]
pub enum Expr {
    Field(&'static str),
    LitInt(i64),
    LitString(&'static str),
    LitBool(bool),
    Null,
    StringLength(&'static str),
    Length(&'static Expr),
    Lower(&'static Expr),
    Upper(&'static Expr),
    Concat(&'static [&'static Expr]),
    Coalesce(&'static [&'static Expr]),
    Add(&'static Expr, &'static Expr),
    Sub(&'static Expr, &'static Expr),
    Mul(&'static Expr, &'static Expr),
    Div(&'static Expr, &'static Expr),
    Eq(&'static Expr, &'static Expr),
    Ne(&'static Expr, &'static Expr),
    Gt(&'static Expr, &'static Expr),
    Gte(&'static Expr, &'static Expr),
    Lt(&'static Expr, &'static Expr),
    Lte(&'static Expr, &'static Expr),
    IfElse {
        cond: &'static Expr,
        then_expr: &'static Expr,
        else_expr: &'static Expr,
    },
    Arg(&'static str),
    Custom(fn(&FieldMap) -> Result<Value>),
}

impl PartialEq for Expr {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Field(a), Self::Field(b)) => a == b,
            (Self::Arg(a), Self::Arg(b)) => a == b,
            (Self::LitInt(a), Self::LitInt(b)) => a == b,
            (Self::LitString(a), Self::LitString(b)) => a == b,
            (Self::LitBool(a), Self::LitBool(b)) => a == b,
            (Self::Null, Self::Null) => true,
            (Self::StringLength(a), Self::StringLength(b)) => a == b,
            (Self::Length(a), Self::Length(b)) => a == b,
            (Self::Lower(a), Self::Lower(b)) => a == b,
            (Self::Upper(a), Self::Upper(b)) => a == b,
            (Self::Concat(a), Self::Concat(b)) => a == b,
            (Self::Coalesce(a), Self::Coalesce(b)) => a == b,
            (Self::Add(a1, a2), Self::Add(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Sub(a1, a2), Self::Sub(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Mul(a1, a2), Self::Mul(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Div(a1, a2), Self::Div(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Eq(a1, a2), Self::Eq(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Ne(a1, a2), Self::Ne(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Gt(a1, a2), Self::Gt(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Gte(a1, a2), Self::Gte(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Lt(a1, a2), Self::Lt(b1, b2)) => a1 == b1 && a2 == b2,
            (Self::Lte(a1, a2), Self::Lte(b1, b2)) => a1 == b1 && a2 == b2,
            (
                Self::IfElse {
                    cond: c1,
                    then_expr: t1,
                    else_expr: e1,
                },
                Self::IfElse {
                    cond: c2,
                    then_expr: t2,
                    else_expr: e2,
                },
            ) => c1 == c2 && t1 == t2 && e1 == e2,
            (Self::Custom(f1), Self::Custom(f2)) => (*f1 as usize) == (*f2 as usize),
            _ => false,
        }
    }
}

impl Eq for Expr {}

#[derive(Clone, Copy, Debug)]
pub struct CalculationDef {
    pub name: &'static str,
    pub ty: AttrType,
    pub expr: Expr,
    pub arguments: &'static [crate::action::ArgumentDef],
}

impl CalculationDef {
    pub const fn new(name: &'static str, ty: AttrType, expr: Expr) -> Self {
        Self {
            name,
            ty,
            expr,
            arguments: &[],
        }
    }

    pub const fn with_arguments(
        name: &'static str,
        ty: AttrType,
        expr: Expr,
        arguments: &'static [crate::action::ArgumentDef],
    ) -> Self {
        Self {
            name,
            ty,
            expr,
            arguments,
        }
    }
}

pub fn eval(expr: &Expr, fields: &FieldMap) -> Result<Value> {
    eval_with_args(expr, fields, &FieldMap::new())
}

pub fn eval_with_args(expr: &Expr, fields: &FieldMap, args: &FieldMap) -> Result<Value> {
    match *expr {
        Expr::Field(name) => Ok(fields.get(name).cloned().unwrap_or(Value::Null)),
        Expr::Arg(name) => Ok(args.get(name).cloned().unwrap_or(Value::Null)),
        Expr::LitInt(n) => Ok(Value::Int(n)),
        Expr::LitString(s) => Ok(Value::String(s.to_string())),
        Expr::LitBool(b) => Ok(Value::Bool(b)),
        Expr::Null => Ok(Value::Null),
        Expr::StringLength(name) => match fields.get(name) {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(Value::String(text)) => Ok(Value::Int(text.chars().count() as i64)),
            Some(other) => Err(Error::TypeMismatch {
                field: name.to_string(),
                expected: "string".into(),
                got: other.type_name().into(),
            }),
        },
        Expr::Length(inner) => match eval_with_args(inner, fields, args)? {
            Value::Null => Ok(Value::Null),
            Value::String(text) => Ok(Value::Int(text.chars().count() as i64)),
            other => Err(Error::TypeMismatch {
                field: "length".into(),
                expected: "string".into(),
                got: other.type_name().into(),
            }),
        },
        Expr::Lower(inner) => match eval_with_args(inner, fields, args)? {
            Value::Null => Ok(Value::Null),
            Value::String(s) => Ok(Value::String(s.to_lowercase())),
            other => Err(Error::TypeMismatch {
                field: "lower".into(),
                expected: "string".into(),
                got: other.type_name().into(),
            }),
        },
        Expr::Upper(inner) => match eval_with_args(inner, fields, args)? {
            Value::Null => Ok(Value::Null),
            Value::String(s) => Ok(Value::String(s.to_uppercase())),
            other => Err(Error::TypeMismatch {
                field: "upper".into(),
                expected: "string".into(),
                got: other.type_name().into(),
            }),
        },
        Expr::Concat(parts) => {
            let mut out = String::new();
            for part in parts {
                match eval_with_args(part, fields, args)? {
                    Value::Null => {}
                    Value::String(s) => out.push_str(&s),
                    Value::Int(n) => out.push_str(&n.to_string()),
                    Value::Bool(b) => out.push_str(&b.to_string()),
                    Value::Uuid(u) => out.push_str(&u.to_string()),
                    _ => {}
                }
            }
            Ok(Value::String(out))
        }
        Expr::Coalesce(parts) => {
            for part in parts {
                let v = eval_with_args(part, fields, args)?;
                if v != Value::Null {
                    return Ok(v);
                }
            }
            Ok(Value::Null)
        }
        Expr::Add(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a + b)),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b))),
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(Error::TypeMismatch {
                field: "add".into(),
                expected: "integer or string".into(),
                got: format!("{} + {}", a.type_name(), b.type_name()),
            }),
        },
        Expr::Sub(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a - b)),
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(Error::TypeMismatch {
                field: "sub".into(),
                expected: "integer".into(),
                got: format!("{} - {}", a.type_name(), b.type_name()),
            }),
        },
        Expr::Mul(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Int(a * b)),
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(Error::TypeMismatch {
                field: "mul".into(),
                expected: "integer".into(),
                got: format!("{} * {}", a.type_name(), b.type_name()),
            }),
        },
        Expr::Div(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => {
                if b == 0 {
                    Ok(Value::Null)
                } else {
                    Ok(Value::Int(a / b))
                }
            }
            (Value::Null, _) | (_, Value::Null) => Ok(Value::Null),
            (a, b) => Err(Error::TypeMismatch {
                field: "div".into(),
                expected: "integer".into(),
                got: format!("{} / {}", a.type_name(), b.type_name()),
            }),
        },
        Expr::Eq(l, r) => {
            let a = eval_with_args(l, fields, args)?;
            let b = eval_with_args(r, fields, args)?;
            Ok(Value::Bool(a == b))
        }
        Expr::Ne(l, r) => {
            let a = eval_with_args(l, fields, args)?;
            let b = eval_with_args(r, fields, args)?;
            Ok(Value::Bool(a != b))
        }
        Expr::Gt(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a > b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Bool(a > b)),
            _ => Ok(Value::Bool(false)),
        },
        Expr::Gte(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a >= b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Bool(a >= b)),
            _ => Ok(Value::Bool(false)),
        },
        Expr::Lt(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a < b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Bool(a < b)),
            _ => Ok(Value::Bool(false)),
        },
        Expr::Lte(l, r) => match (eval_with_args(l, fields, args)?, eval_with_args(r, fields, args)?) {
            (Value::Int(a), Value::Int(b)) => Ok(Value::Bool(a <= b)),
            (Value::String(a), Value::String(b)) => Ok(Value::Bool(a <= b)),
            _ => Ok(Value::Bool(false)),
        },
        Expr::IfElse { cond, then_expr, else_expr } => {
            match eval_with_args(cond, fields, args)? {
                Value::Bool(true) => eval_with_args(then_expr, fields, args),
                _ => eval_with_args(else_expr, fields, args),
            }
        }
        Expr::Custom(f) => f(fields),
    }
}

pub fn apply_named(resource: &ResourceDef, row: &mut FieldMap, name: &str) -> Result<()> {
    apply_named_with_args(resource, row, name, &FieldMap::new())
}

pub fn apply_named_with_args(
    resource: &ResourceDef,
    row: &mut FieldMap,
    name: &str,
    args: &FieldMap,
) -> Result<()> {
    if let Some(calc) = resource.calculation(name) {
        let value = eval_with_args(&calc.expr, row, args)?;
        row.insert(calc.name.to_string(), value);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fields;

    #[test]
    fn string_length_counts_chars() {
        let row = fields! { "subject" => "café" };
        assert_eq!(
            eval(&Expr::StringLength("subject"), &row).unwrap(),
            Value::Int(4)
        );
    }

    #[test]
    fn string_length_of_missing_is_null() {
        let empty = fields! {};
        assert_eq!(
            eval(&Expr::StringLength("subject"), &empty).unwrap(),
            Value::Null
        );
    }
}
