use uuid::Uuid;

use crate::data_layer::Sort;
use crate::filter::Filter;
use crate::resource::{Resource, ResourceDef};
use crate::value::Value;

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct KeysetCursor {
    pub id: Uuid,
    pub values: Vec<(String, Value)>,
}

impl KeysetCursor {
    pub fn encode(&self) -> String {
        use base64::prelude::*;
        let json = serde_json::to_string(self).unwrap_or_default();
        BASE64_URL_SAFE_NO_PAD.encode(json.as_bytes())
    }

    pub fn decode(cursor_str: &str) -> Option<Self> {
        use base64::prelude::*;
        if let Ok(bytes) = BASE64_URL_SAFE_NO_PAD.decode(cursor_str.as_bytes())
            && let Ok(cursor) = serde_json::from_slice::<Self>(&bytes)
        {
            return Some(cursor);
        }
        if let Ok(cursor) = serde_json::from_str::<Self>(cursor_str) {
            return Some(cursor);
        }
        if let Ok(id) = Uuid::parse_str(cursor_str) {
            return Some(Self {
                id,
                values: Vec::new(),
            });
        }
        None
    }
}

/// The sort a keyset page reads in, as Ash makes it stable: `sort`, then the primary key,
/// unless the sort already orders by the primary key or every key of an identity, so no
/// two records share a keyset.
pub fn keyset_sort(resource: &ResourceDef, mut sort: Vec<Sort>) -> Vec<Sort> {
    let sorted_on = |keys: &[&str]| keys.iter().all(|key| sort.iter().any(|s| s.field == *key));
    let primary_key: Vec<&str> = resource
        .attributes
        .iter()
        .filter(|attr| attr.primary_key)
        .map(|attr| attr.name)
        .collect();
    if sorted_on(&primary_key) || resource.identities.iter().any(|identity| sorted_on(identity.keys)) {
        return sort;
    }
    for key in primary_key {
        sort.push(Sort {
            field: key.to_string(),
            descending: false,
        });
    }
    sort
}

/// The filter for the records after (or before) a keyset: `sorts` holds each sort
/// field, the keyset's value for it, and whether it sorts descending.
pub fn build_keyset_filter(sorts: &[(String, Value, bool)], is_after: bool) -> Option<Filter> {
    if sorts.is_empty() {
        return None;
    }
    let (field, val, desc) = &sorts[0];
    let cond = if is_after {
        if !desc {
            Filter::Gt(field.clone(), val.clone())
        } else {
            Filter::Lt(field.clone(), val.clone())
        }
    } else if !desc {
        Filter::Lt(field.clone(), val.clone())
    } else {
        Filter::Gt(field.clone(), val.clone())
    };

    if sorts.len() == 1 {
        Some(cond)
    } else {
        let eq = Filter::Eq(field.clone(), val.clone());
        let rest = build_keyset_filter(&sorts[1..], is_after)?;
        Some(Filter::or([cond, Filter::and([eq, rest])]))
    }
}

pub(crate) fn cursor_for_record<R: Resource>(record: &R, sort: &[Sort]) -> String {
    let fields = R::to_fields(record);
    let mut values = Vec::with_capacity(sort.len());
    for s in sort {
        let val = fields.get(&s.field).cloned().unwrap_or(Value::Null);
        values.push((s.field.clone(), val));
    }
    KeysetCursor {
        id: record.id(),
        values,
    }
    .encode()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub results: Vec<T>,
    pub has_more: bool,
    pub limit: usize,
    pub offset: Option<usize>,
    pub total_count: Option<usize>,
    pub after: Option<String>,
    pub before: Option<String>,
}

impl<T> Page<T> {
    pub fn results(&self) -> &[T] {
        &self.results
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }

    pub fn total_count(&self) -> Option<usize> {
        self.total_count
    }

    pub fn after(&self) -> Option<&str> {
        self.after.as_deref()
    }

    pub fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }
}
