//! Names on the wire, camelCase, and in the resource, snake_case.

use std::collections::HashMap;

use ash_core::{ActionDef, ResourceDef};
use serde_json::Value as Json;

use crate::types::to_camel_case;

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



/// How clients name a resource's fields and its actions' arguments: their names in
/// camelCase, or the names AshTypescript's `field_names` and `argument_names` map them to
/// (in camelCase too), for names that don't survive the round trip, like `line_1`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Names {
    fields: HashMap<&'static str, Vec<(&'static str, &'static str)>>,
    arguments: HashMap<(&'static str, &'static str), Vec<(&'static str, &'static str)>>,
}


impl Names {
    pub(crate) fn map_fields(&mut self, resource: &'static str, names: &[(&'static str, &'static str)]) {
        self.fields.entry(resource).or_default().extend_from_slice(names);
    }

    pub(crate) fn map_arguments(&mut self, resource: &'static str, action: &'static str, names: &[(&'static str, &'static str)]) {
        self.arguments.entry((resource, action)).or_default().extend_from_slice(names);
    }

    /// Whether `resource`'s field `field` is mapped.
    pub(crate) fn maps_field(&self, resource: &str, field: &str) -> bool {
        self.fields.get(resource).is_some_and(|names| names.iter().any(|(own, _)| *own == field))
    }

    pub(crate) fn maps_argument(&self, resource: &str, action: &str, argument: &str) -> bool {
        self.arguments.get(&(resource, action)).is_some_and(|names| names.iter().any(|(own, _)| *own == argument))
    }

    /// The name a client knows `resource`'s field `field` by.
    pub(crate) fn field(&self, resource: &ResourceDef, field: &str) -> String {
        let mapped = self.fields.get(resource.name).and_then(|names| names.iter().find(|(own, _)| *own == field));
        to_camel_case(mapped.map_or(field, |(_, name)| name))
    }

    /// The field of `resource` a client names `given`: a mapped one, one whose name in
    /// camelCase it is, or else its name in snake_case.
    pub(crate) fn field_named(&self, resource: &ResourceDef, given: &str) -> String {
        if let Some((own, _)) = self.fields.get(resource.name).and_then(|names| names.iter().find(|(_, name)| to_camel_case(name) == given || *name == given)) {
            return own.to_string();
        }
        field_names(resource)
            .find(|name| !self.maps_field(resource.name, name) && to_camel_case(name) == given)
            .map_or_else(|| snake(given), str::to_string)
    }

    /// The name a client knows argument `argument` of `resource`'s `action` by.
    pub(crate) fn argument(&self, resource: &ResourceDef, action: &str, argument: &str) -> String {
        let mapped = self.arguments.get(&(resource.name, action)).and_then(|names| names.iter().find(|(own, _)| *own == argument));
        to_camel_case(mapped.map_or(argument, |(_, name)| name))
    }

    /// A key of `action`'s input as the action takes it: an argument's name, or a field's.
    pub(crate) fn input_named(&self, resource: &ResourceDef, action: &ActionDef, given: &str) -> String {
        if let Some((own, _)) = self
            .arguments
            .get(&(resource.name, action.name))
            .and_then(|names| names.iter().find(|(_, name)| to_camel_case(name) == given || *name == given))
        {
            return own.to_string();
        }
        if let Some(arg) = action.arguments.iter().find(|arg| !self.maps_argument(resource.name, action.name, arg.name) && to_camel_case(arg.name) == given) {
            return arg.name.to_string();
        }
        self.field_named(resource, given)
    }

    /// An action's input as the client sent it, its keys as the action takes them.
    pub(crate) fn input(&self, resource: &ResourceDef, action: &ActionDef, json: &Json) -> Json {
        match json {
            Json::Object(map) => Json::Object(map.iter().map(|(k, v)| (self.input_named(resource, action, k), v.clone())).collect()),
            other => other.clone(),
        }
    }

    /// A filter as the client sent it, its field names as `resource` and the resources
    /// its relationships reach know them, its operators in snake_case.
    pub(crate) fn filter(&self, resource: &ResourceDef, json: &Json) -> Json {
        match json {
            Json::Object(map) => Json::Object(
                map.iter()
                    .map(|(key, value)| match key.as_str() {
                        "and" | "or" | "not" => (key.clone(), match value {
                            Json::Array(items) => Json::Array(items.iter().map(|item| self.filter(resource, item)).collect()),
                            other => self.filter(resource, other),
                        }),
                        _ => {
                            let field = self.field_named(resource, key);
                            let value = match resource.relationship(&field) {
                                Some(rel) => self.filter((rel.destination)(), value),
                                None => snake_keys(value),
                            };
                            (field, value)
                        }
                    })
                    .collect(),
            ),
            Json::Array(items) => Json::Array(items.iter().map(|item| self.filter(resource, item)).collect()),
            other => other.clone(),
        }
    }

    /// A sort as the client sent it (`"-insertedAt,id"`), its fields as `resource` knows
    /// them.
    pub(crate) fn sort(&self, resource: &ResourceDef, text: &str) -> String {
        text.split(',')
            .map(|part| {
                let part = part.trim();
                let field = part.trim_start_matches(['+', '-']);
                format!("{}{}", &part[..part.len() - field.len()], self.field_named(resource, field))
            })
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Every name a client may select of `resource`.
fn field_names(resource: &ResourceDef) -> impl Iterator<Item = &'static str> + '_ {
    resource
        .attributes
        .iter()
        .map(|attr| attr.name)
        .chain(resource.calculations.iter().map(|calc| calc.name))
        .chain(resource.aggregates.iter().map(|agg| agg.name))
        .chain(resource.relationships.iter().map(|rel| rel.name))
}
