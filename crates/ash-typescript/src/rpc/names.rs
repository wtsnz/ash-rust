//! Names on the wire, camelCase, and in the resource, snake_case.

use serde_json::Value as Json;

/// `requesterEmail` → `requester_email`.
pub(crate) fn snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            out.push('_');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// A filter as the client sent it, its keys in snake_case: field names and operators at
/// every level.
pub(crate) fn snake_keys(json: &Json) -> Json {
    match json {
        Json::Object(map) => Json::Object(map.iter().map(|(k, v)| (snake(k), snake_keys(v))).collect()),
        Json::Array(items) => Json::Array(items.iter().map(snake_keys).collect()),
        other => other.clone(),
    }
}

/// An action's input as the client sent it, its field names in snake_case. Their values
/// are left as they are: a map's keys are its own, as AshTypescript leaves them.
pub(crate) fn snake_input(json: &Json) -> Json {
    match json {
        Json::Object(map) => Json::Object(map.iter().map(|(k, v)| (snake(k), v.clone())).collect()),
        other => other.clone(),
    }
}

/// `"-insertedAt,id"` → `"-inserted_at,id"`.
pub(crate) fn snake_sort(text: &str) -> String {
    text.split(',').map(|part| snake(part.trim())).collect::<Vec<_>>().join(",")
}
