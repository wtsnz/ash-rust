use std::marker::PhantomData;

use crate::filter::Filter;
use crate::value::Value;

/// Stored attribute on `R`, value type `T`.
#[derive(Clone, Copy, Debug)]
pub struct Attr<R, T> {
    name: &'static str,
    _ty: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Attr<R, T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _ty: PhantomData,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

impl<R, T> Attr<R, T>
where
    T: Into<Value>,
{
    pub fn eq(self, value: impl Into<T>) -> Filter {
        Filter::eq(self.name, value.into())
    }

    pub fn ne(self, value: impl Into<T>) -> Filter {
        Filter::ne(self.name, value.into())
    }

    pub fn gt(self, value: impl Into<T>) -> Filter {
        Filter::gt(self.name, value.into())
    }

    pub fn gte(self, value: impl Into<T>) -> Filter {
        Filter::gte(self.name, value.into())
    }

    pub fn lt(self, value: impl Into<T>) -> Filter {
        Filter::lt(self.name, value.into())
    }

    pub fn lte(self, value: impl Into<T>) -> Filter {
        Filter::lte(self.name, value.into())
    }

    pub fn is_nil(self) -> Filter {
        Filter::is_nil(self.name)
    }
}

/// Calculation on `R`, value type `T`.
#[derive(Clone, Copy, Debug)]
pub struct Calc<R, T> {
    name: &'static str,
    _ty: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Calc<R, T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _ty: PhantomData,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

impl<R, T> Calc<R, T>
where
    T: Into<Value>,
{
    pub fn eq(self, value: impl Into<T>) -> Filter {
        Filter::eq(self.name, value.into())
    }

    pub fn gt(self, value: impl Into<T>) -> Filter {
        Filter::gt(self.name, value.into())
    }

    pub fn gte(self, value: impl Into<T>) -> Filter {
        Filter::gte(self.name, value.into())
    }

    pub fn lt(self, value: impl Into<T>) -> Filter {
        Filter::lt(self.name, value.into())
    }

    pub fn lte(self, value: impl Into<T>) -> Filter {
        Filter::lte(self.name, value.into())
    }
}

/// Relationship from `R` to `Dest`.
#[derive(Clone, Copy, Debug)]
pub struct Relation<R, Dest> {
    name: &'static str,
    _ty: PhantomData<fn() -> (R, Dest)>,
}

impl<R, Dest> Relation<R, Dest> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _ty: PhantomData,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

pub trait FieldName<R> {
    fn as_field(&self) -> &str;
}

impl<R> FieldName<R> for &str {
    fn as_field(&self) -> &str {
        self
    }
}

impl<R> FieldName<R> for String {
    fn as_field(&self) -> &str {
        self
    }
}

impl<R, T> FieldName<R> for Attr<R, T> {
    fn as_field(&self) -> &str {
        self.name
    }
}

impl<R, T> FieldName<R> for Calc<R, T> {
    fn as_field(&self) -> &str {
        self.name
    }
}

pub trait CalcName<R> {
    fn as_calc(&self) -> &str;
}

impl<R> CalcName<R> for &str {
    fn as_calc(&self) -> &str {
        self
    }
}

impl<R> CalcName<R> for String {
    fn as_calc(&self) -> &str {
        self
    }
}

impl<R, T> CalcName<R> for Calc<R, T> {
    fn as_calc(&self) -> &str {
        self.name
    }
}

pub trait RelName<R> {
    fn as_rel(&self) -> &str;
}

impl<R> RelName<R> for &str {
    fn as_rel(&self) -> &str {
        self
    }
}

impl<R> RelName<R> for String {
    fn as_rel(&self) -> &str {
        self
    }
}

impl<R, Dest> RelName<R> for Relation<R, Dest> {
    fn as_rel(&self) -> &str {
        self.name
    }
}

/// Aggregate on `R`, value type `T`.
#[derive(Clone, Copy, Debug)]
pub struct Aggregate<R, T> {
    name: &'static str,
    _ty: PhantomData<fn() -> (R, T)>,
}

impl<R, T> Aggregate<R, T> {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _ty: PhantomData,
        }
    }

    pub const fn name(self) -> &'static str {
        self.name
    }
}

impl<R, T> Aggregate<R, T>
where
    T: Into<Value>,
{
    pub fn eq(self, value: impl Into<T>) -> Filter {
        Filter::eq(self.name, value.into())
    }

    pub fn ne(self, value: impl Into<T>) -> Filter {
        Filter::ne(self.name, value.into())
    }

    pub fn gt(self, value: impl Into<T>) -> Filter {
        Filter::gt(self.name, value.into())
    }

    pub fn gte(self, value: impl Into<T>) -> Filter {
        Filter::gte(self.name, value.into())
    }

    pub fn lt(self, value: impl Into<T>) -> Filter {
        Filter::lt(self.name, value.into())
    }

    pub fn lte(self, value: impl Into<T>) -> Filter {
        Filter::lte(self.name, value.into())
    }

    pub fn is_nil(self) -> Filter {
        Filter::is_nil(self.name)
    }
}

impl<R, T> FieldName<R> for Aggregate<R, T> {
    fn as_field(&self) -> &str {
        self.name
    }
}

pub trait AggregateName<R> {
    fn as_aggregate(&self) -> &str;
}

impl<R> AggregateName<R> for &str {
    fn as_aggregate(&self) -> &str {
        self
    }
}

impl<R> AggregateName<R> for String {
    fn as_aggregate(&self) -> &str {
        self
    }
}

impl<R, T> AggregateName<R> for Aggregate<R, T> {
    fn as_aggregate(&self) -> &str {
        self.name
    }
}

