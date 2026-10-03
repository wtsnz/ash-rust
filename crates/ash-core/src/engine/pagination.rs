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
            guard: None,
        });
    }
    sort
}

/// The filter for the records after (or before) a keyset: `values` holds the keyset's
/// value for each of `sorts`. As Ash orders them, nulls sort last ascending and first
/// descending, so a null value, or a field that may hold one, needs its own branch: past
/// a null only nulls follow (or nothing precedes), and past a value the nulls beyond it
/// follow too. A guarded sort's field reads as null wherever its guard doesn't hold.
pub fn build_keyset_filter(resource: &ResourceDef, sorts: &[Sort], values: &[Value], is_after: bool) -> Option<Filter> {
    let (sort, value) = (sorts.first()?, values.first()?);
    // Moving towards the end of an ascending sort (or the start of a descending one),
    // values grow and the nulls lie ahead.
    let forward = is_after != sort.descending;
    let nullable = sort.guard.is_some()
        || resource.attribute(&sort.field).is_none_or(|attr| attr.allow_nil);
    let field = sort.field.clone();
    let guarded = |filter: Filter| match &sort.guard {
        Some(guard) => Filter::and([guard.clone(), filter]),
        None => filter,
    };
    let is_nil = || match &sort.guard {
        Some(guard) => Filter::or([!guard.clone(), Filter::IsNil(field.clone())]),
        None => Filter::IsNil(field.clone()),
    };
    let (beyond, tied) = if value.is_null() {
        let beyond = if forward { Filter::False } else { !is_nil() };
        (beyond, is_nil())
    } else {
        let past = guarded(if forward {
            Filter::Gt(field.clone(), value.clone())
        } else {
            Filter::Lt(field.clone(), value.clone())
        });
        let beyond = if forward && nullable { Filter::or([past, is_nil()]) } else { past };
        (beyond, guarded(Filter::Eq(field.clone(), value.clone())))
    };
    match build_keyset_filter(resource, &sorts[1..], &values[1..], is_after) {
        None => Some(beyond),
        Some(rest) => Some(Filter::or([beyond, Filter::and([tied, rest])])),
    }
}

/// The keyset's value for each of `sorts`: the primary key's from its id when the keyset
/// doesn't carry it.
pub fn keyset_values(resource: &ResourceDef, cursor: &KeysetCursor, sorts: &[Sort]) -> Vec<Value> {
    let pk = resource.primary_key().map(|attr| attr.name);
    sorts
        .iter()
        .map(|sort| match cursor.values.iter().find(|(field, _)| *field == sort.field) {
            Some((_, value)) => value.clone(),
            None if Some(sort.field.as_str()) == pk => Value::Uuid(cursor.id),
            None => Value::Null,
        })
        .collect()
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
