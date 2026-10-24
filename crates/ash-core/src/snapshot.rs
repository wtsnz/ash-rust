//! Filters evaluated against a committed record snapshot, without querying relations.

use crate::{Error, FieldMap, Filter, ResourceDef, Result, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterMatch {
    Match,
    NoMatch,
    NeedsData,
}

impl Filter {
    /// Missing dependencies are distinct from SQL null or a false comparison.
    pub fn snapshot_match(&self, resource: &ResourceDef, fields: &FieldMap) -> FilterMatch {
        if needs_data(self, resource, fields) {
            FilterMatch::NeedsData
        } else if self.eval(Some(resource), fields) == Some(true) {
            FilterMatch::Match
        } else {
            FilterMatch::NoMatch
        }
    }

    /// Notification snapshots contain stored attributes only. Relationship, aggregate
    /// and calculation predicates require an event-query implementation.
    pub fn validate_snapshot(&self, resource: &ResourceDef) -> Result<()> {
        match self {
            Self::Related { .. } | Self::Traversal(_) => Err(Error::Invalid(
                "subscription filters cannot depend on relationships".into(),
            )),
            Self::Client(_) => Err(Error::Invalid(
                "subscription filter requires read planning".into(),
            )),
            Self::And(parts) | Self::Or(parts) => parts
                .iter()
                .try_for_each(|part| part.validate_snapshot(resource)),
            Self::Not(inner) | Self::Projected(inner) => inner.validate_snapshot(resource),
            leaf => {
                let mut fields = Vec::new();
                leaf.collect_fields(&mut fields);
                if fields
                    .iter()
                    .any(|field| resource.attribute(field).is_none())
                {
                    Err(Error::Invalid(
                        "subscription filters cannot depend on calculations or aggregates".into(),
                    ))
                } else {
                    Ok(())
                }
            }
        }
    }
}

fn needs_data(filter: &Filter, resource: &ResourceDef, fields: &FieldMap) -> bool {
    match filter {
        Filter::Client(_) => true,
        Filter::Traversal(node) => {
            // A snapshot does not carry the join row needed to establish its scope.
            node.join_filter().is_some()
                || related_needs_data(node.relationship(), node.filter(), resource, fields)
        }
        Filter::Related {
            relationship,
            filter,
        } => related_needs_data(relationship, filter, resource, fields),
        Filter::And(parts) | Filter::Or(parts) => {
            parts.iter().any(|part| needs_data(part, resource, fields))
        }
        Filter::Not(inner) | Filter::Projected(inner) => needs_data(inner, resource, fields),
        leaf => {
            let mut names = Vec::new();
            leaf.collect_fields(&mut names);
            names.iter().any(|name| !fields.contains_key(*name))
        }
    }
}

fn related_needs_data(
    name: &str,
    filter: &Filter,
    resource: &ResourceDef,
    fields: &FieldMap,
) -> bool {
    let Some(rel) = resource.relationship(name) else {
        return true;
    };
    let dest = (rel.destination)();
    match fields.get(name) {
        Some(Value::Null) => false,
        Some(Value::Map(row)) => needs_data(filter, dest, row),
        Some(Value::Array(rows)) => rows.iter().any(|row| match row {
            Value::Map(row) => needs_data(filter, dest, row),
            _ => true,
        }),
        _ => true,
    }
}
