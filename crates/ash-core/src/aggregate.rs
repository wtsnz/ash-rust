use crate::resource::AttrType;
use crate::value::ConstValue;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    Count,
    Exists,
    First { field: &'static str },
    Sum { field: &'static str },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AggregateFilter {
    Eq(&'static str, ConstValue),
    Ne(&'static str, ConstValue),
}

#[derive(Clone, Copy, Debug)]
pub struct AggregateDef {
    pub name: &'static str,
    pub relationship: &'static str,
    pub kind: AggregateKind,
    pub filter: Option<AggregateFilter>,
    pub ty: AttrType,
}

impl AggregateDef {
    pub const fn count(name: &'static str, relationship: &'static str) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Count,
            filter: None,
            ty: AttrType::Integer,
        }
    }

    pub const fn count_where(
        name: &'static str,
        relationship: &'static str,
        filter: AggregateFilter,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Count,
            filter: Some(filter),
            ty: AttrType::Integer,
        }
    }

    pub const fn exists(name: &'static str, relationship: &'static str) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Exists,
            filter: None,
            ty: AttrType::Boolean,
        }
    }

    pub const fn exists_where(
        name: &'static str,
        relationship: &'static str,
        filter: AggregateFilter,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Exists,
            filter: Some(filter),
            ty: AttrType::Boolean,
        }
    }

    pub const fn first(
        name: &'static str,
        relationship: &'static str,
        field: &'static str,
        ty: AttrType,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::First { field },
            filter: None,
            ty,
        }
    }

    pub const fn first_where(
        name: &'static str,
        relationship: &'static str,
        field: &'static str,
        ty: AttrType,
        filter: AggregateFilter,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::First { field },
            filter: Some(filter),
            ty,
        }
    }

    pub const fn sum(
        name: &'static str,
        relationship: &'static str,
        field: &'static str,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Sum { field },
            filter: None,
            ty: AttrType::Integer,
        }
    }

    pub const fn sum_where(
        name: &'static str,
        relationship: &'static str,
        field: &'static str,
        filter: AggregateFilter,
    ) -> Self {
        Self {
            name,
            relationship,
            kind: AggregateKind::Sum { field },
            filter: Some(filter),
            ty: AttrType::Integer,
        }
    }
}
