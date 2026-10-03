//! What a request's `fields` select, as AshTypescript's field selector reads them, and
//! the errors it finds in them, in the order it finds them.
//!
//! Each level of a selection is a list. An item is a field's name; or an object naming
//! fields, each with what it selects in turn:
//!
//! - a relationship, a list of its destination's fields, or an envelope of them with its
//!   own read: `{"comments": {"fields": [...], "filter": ..., "sort": ..., "limit": 3}}`,
//!   or `"page"` for a page of them;
//! - a calculation that takes arguments, `{"label": {"args": {...}}}`.
//!
//! Every load (a relationship, calculation or aggregate) passes the RPC action's
//! `allowed_loads` or `denied_loads` first.

use std::collections::HashMap;

use ash_core::input::{filter_input, sort_input, value_input};
use ash_core::{Actor, FieldMap, RelKind, RelatedQuery, ResourceDef};
use serde_json::{Map, Value as Json};

use super::error::Failure;
use super::names::{snake, snake_keys, snake_sort};

/// What a request selects of a resource.
#[derive(Clone, Debug, Default)]
pub(crate) struct Selection {
    /// Attributes, aggregates and calculations, by name, in the order given.
    pub fields: Vec<String>,
    /// The arguments a calculation that takes them is loaded with.
    pub calculation_args: HashMap<String, FieldMap>,
    pub relationships: Vec<(String, Nested)>,
}

/// A relationship's selection, how its rows are read, and whether they come as a page.
#[derive(Clone, Debug)]
pub(crate) struct Nested {
    pub selection: Selection,
    pub query: RelatedQuery,
    pub page: Option<NestedPage>,
}

/// A page of a relationship's rows for each record: `limit` and `offset`, or a keyset,
/// and whether to count them.
#[derive(Clone, Debug, Default)]
pub(crate) struct NestedPage {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub count: bool,
    pub keyset: bool,
}

/// The RPC action's options that decide what a selection may do.
pub(crate) struct Rules<'a> {
    pub actor: Option<&'a Actor>,
    pub enable_filter: bool,
    pub enable_sort: bool,
    pub loads: &'a LoadRestrictions,
}

/// An RPC action's `allowed_loads` or `denied_loads`, as paths of field names.
#[derive(Clone, Debug, Default)]
pub enum LoadRestrictions {
    #[default]
    None,
    /// Only these loads, and the loads on the way to them.
    Allow(Vec<Vec<String>>),
    /// None of these loads, nor any under them.
    Deny(Vec<Vec<String>>),
}

impl LoadRestrictions {
    fn check(&self, path: &[String], field: &str) -> Result<(), Failure> {
        let mut full = path.to_vec();
        full.push(field.to_string());
        let shown = || vec![full.join(".")];
        match self {
            Self::None => Ok(()),
            Self::Allow(allowed) if allowed.iter().any(|a| *a == full || a.starts_with(&full)) => Ok(()),
            Self::Allow(_) => Err(Failure::load_restricted(false, shown())),
            Self::Deny(denied) if denied.iter().any(|d| full.starts_with(d)) => Err(Failure::load_restricted(true, shown())),
            Self::Deny(_) => Ok(()),
        }
    }
}

const QUERY_OPTION_KEYS: [&str; 5] = ["page", "filter", "sort", "limit", "offset"];

/// What an object in a selection holds for a field.
enum Spec<'a> {
    /// A list of fields, or something else, which a field that can't hold it fails on.
    Fields,
    /// Query options for a relationship: `args` (which it mustn't have), its `fields`,
    /// and `page`, `filter`, `sort`, `limit`, `offset`.
    QueryOptions { args: Option<&'a Json>, fields: Option<&'a Json>, options: &'a Map<String, Json> },
    /// `args` for a calculation, with `fields` for one of a type that has them; or bare
    /// `fields`, a relationship's.
    Args { args: Option<&'a Json>, fields: Option<&'a Json> },
}

fn classify(spec: &Json) -> Spec<'_> {
    let Json::Object(map) = spec else {
        return Spec::Fields;
    };
    let args = map.get("args").filter(|args| !args.is_null());
    let fields = map.get("fields");
    if QUERY_OPTION_KEYS.iter().any(|key| map.contains_key(*key)) {
        Spec::QueryOptions { args, fields, options: map }
    } else if args.is_some() || fields.is_some() {
        Spec::Args { args, fields }
    } else {
        Spec::Fields
    }
}

/// What kind of field `name` is on `resource`, as AshTypescript names the kinds.
fn kind(resource: &ResourceDef, name: &str) -> Option<&'static str> {
    if resource.attribute(name).is_some() {
        Some("attribute")
    } else if resource.calculation(name).is_some() {
        Some("calculation")
    } else if resource.aggregate(name).is_some() {
        Some("aggregate")
    } else if resource.relationship(name).is_some() {
        Some("relationship")
    } else {
        None
    }
}

impl Selection {
    /// The selection `fields` makes of `resource`, at `path` (the relationships it's
    /// under).
    pub(crate) fn parse(resource: &'static ResourceDef, fields: &Json, path: &[String], rules: &Rules) -> Result<Self, Failure> {
        let items = match fields {
            Json::Array(items) => items,
            other => return Err(Failure::invalid_fields_type(other)),
        };
        check_duplicates(items, path)?;
        let mut selection = Selection::default();
        for item in items {
            match item {
                Json::String(name) => selection.simple(resource, &snake(name), path, rules)?,
                Json::Object(map) => {
                    for (name, spec) in map {
                        selection.nested(resource, &snake(name), spec, path, rules)?;
                    }
                }
                other => return Err(Failure::invalid_field_format(path, other)),
            }
        }
        Ok(selection)
    }

    /// A field named alone.
    fn simple(&mut self, resource: &'static ResourceDef, name: &str, path: &[String], rules: &Rules) -> Result<(), Failure> {
        match kind(resource, name) {
            None => Err(Failure::unknown_field(path, name, resource)),
            Some("relationship") => Err(Failure::requires_field_selection(path, name, "relationship")),
            Some("calculation") if !resource.calculation(name).unwrap().arguments.is_empty() => {
                Err(Failure::calculation_requires_args(path, name))
            }
            Some("attribute") => {
                self.fields.push(name.to_string());
                Ok(())
            }
            Some(_) => {
                rules.loads.check(path, name)?;
                self.fields.push(name.to_string());
                Ok(())
            }
        }
    }

    /// A field named with what it selects.
    fn nested(&mut self, resource: &'static ResourceDef, name: &str, spec: &Json, path: &[String], rules: &Rules) -> Result<(), Failure> {
        match classify(spec) {
            Spec::QueryOptions { args, fields, options } => self.query_options(resource, name, args, fields, options, path, rules),
            Spec::Args { args, fields } if kind(resource, name) == Some("relationship") => {
                // A relationship's bare `fields` envelope, or one with `args`, which it
                // doesn't take.
                self.query_options(resource, name, args, fields, &Map::new(), path, rules)
            }
            Spec::Args { args, fields } => self.calculation_with_args(resource, name, args, fields, path, rules),
            Spec::Fields => self.nested_fields(resource, name, spec, path, rules),
        }
    }

    /// A field with a list of fields (or something else) of its own: a relationship's.
    fn nested_fields(&mut self, resource: &'static ResourceDef, name: &str, spec: &Json, path: &[String], rules: &Rules) -> Result<(), Failure> {
        match kind(resource, name) {
            None => Err(Failure::unknown_field(path, name, resource)),
            Some("calculation") if !resource.calculation(name).unwrap().arguments.is_empty() => {
                Err(Failure::invalid_calculation_args(path, name))
            }
            Some("aggregate") => Err(Failure::invalid_field_selection(path, name, ":aggregate")),
            Some("calculation") if spec.is_object() => Err(Failure::invalid_calculation_args(path, name)),
            Some("calculation" | "attribute") => Err(Failure::field_does_not_support_nesting(path, name)),
            _ => {
                let Json::Array(items) = spec else {
                    return Err(Failure::unsupported_field_combination(path, name, "relationship", spec));
                };
                if items.is_empty() {
                    return Err(Failure::requires_field_selection(path, name, "relationship"));
                }
                let dest = (resource.relationship(name).unwrap().destination)();
                let selection = Selection::parse(dest, spec, &child(path, name), rules)?;
                rules.loads.check(path, name)?;
                self.relationship(name, dest, selection, RelatedQuery::default(), None);
                Ok(())
            }
        }
    }

    /// A relationship read with its own options, validated as AshTypescript does: a
    /// to-many relationship, without `args`, its `page` one its read pages, its filter
    /// and sort allowed, `page` or bare `limit`/`offset`, and fields to select.
    #[allow(clippy::too_many_arguments)]
    fn query_options(
        &mut self,
        resource: &'static ResourceDef,
        name: &str,
        args: Option<&Json>,
        fields: Option<&Json>,
        options: &Map<String, Json>,
        path: &[String],
        rules: &Rules,
    ) -> Result<(), Failure> {
        let Some(rel) = resource.relationship(name).filter(|_| kind(resource, name) == Some("relationship")) else {
            return Err(match kind(resource, name) {
                None => Failure::unknown_field(path, name, resource),
                Some(kind) => Failure::query_opts_on_non_relationship(path, name, kind),
            });
        };
        if !matches!(rel.kind, RelKind::HasMany | RelKind::ManyToMany) {
            return Err(Failure::query_opts_on_to_one(path, name));
        }
        if args.is_some() {
            return Err(Failure::args_and_query_opts_combined(path, name));
        }
        let dest = (rel.destination)();
        let pagination = dest.primary_read().and_then(|read| read.pagination);
        let given = |key: &str| options.get(key).filter(|value| !value.is_null());
        if given("page").is_some() && pagination.is_none() {
            return Err(Failure::nested_pagination_not_supported(path, name));
        }
        if given("filter").is_some() && !rules.enable_filter {
            return Err(Failure::option_not_supported("filter", "disabled", Some((path, name))));
        }
        if given("sort").is_some() && !rules.enable_sort {
            return Err(Failure::option_not_supported("sort", "disabled", Some((path, name))));
        }
        if given("page").is_some() && (given("limit").is_some() || given("offset").is_some()) {
            return Err(Failure::page_and_limit_offset_combined(path, name));
        }
        let fields = match fields {
            Some(Json::Array(items)) if !items.is_empty() => fields.unwrap(),
            Some(other @ (Json::Object(_) | Json::String(_) | Json::Number(_) | Json::Bool(_))) => {
                return Err(Failure::unsupported_field_combination(path, name, "relationship", other));
            }
            _ => return Err(Failure::requires_field_selection(path, name, "relationship")),
        };
        let at = child(path, name);
        let selection = Selection::parse(dest, fields, &at, rules)?;
        let page = match given("page") {
            None => None,
            Some(page) => Some(nested_page(page, pagination.unwrap(), path, name)?),
        };
        let filter = match given("filter") {
            None => None,
            Some(filter) => {
                let filter = filter_input(dest, &snake_keys(filter)).map_err(|e| failure(&e).under("filter"))?;
                Some(ash_core::guard_input_filter(dest, rules.actor, filter).map_err(|e| failure(&e))?)
            }
        };
        let sort = match given("sort") {
            None => Vec::new(),
            Some(sort) => {
                let text = sort_text(sort);
                let sort = sort_input(dest, &snake_sort(&text)).map_err(|e| failure(&e).under("sort"))?;
                ash_core::guard_input_sort(dest, rules.actor, sort).map_err(|e| failure(&e))?
            }
        };
        let number = |key: &str| given(key).and_then(Json::as_u64).map(|n| n as usize);
        let query = RelatedQuery { filter, sort, limit: number("limit"), offset: number("offset"), ..RelatedQuery::default() };
        rules.loads.check(path, name)?;
        self.relationship(name, dest, selection, query, page);
        Ok(())
    }

    /// A calculation loaded with arguments (`{"label": {"args": {...}}}`), validated as
    /// AshTypescript does: arguments given where it takes them, all those it requires,
    /// and no fields to select of a calculation whose type has none.
    fn calculation_with_args(
        &mut self,
        resource: &'static ResourceDef,
        name: &str,
        args: Option<&Json>,
        fields: Option<&Json>,
        path: &[String],
        rules: &Rules,
    ) -> Result<(), Failure> {
        let Some(calc) = resource.calculation(name) else {
            return Err(Failure::unknown_field(path, name, resource));
        };
        let accepts = !calc.arguments.is_empty();
        let requires = calc.arguments.iter().any(|arg| !arg.allow_nil);
        let given: Option<&Map<String, Json>> = args.and_then(Json::as_object);
        let non_empty = given.is_some_and(|args| !args.is_empty());
        // AshTypescript's rules, for calculations whose types have no fields: arguments
        // given where it takes them and only there, and those it requires.
        if accepts != args.is_some() || (requires && !non_empty) {
            return Err(Failure::invalid_calculation_args(path, name));
        }
        if fields.is_some_and(|fields| !fields.is_null()) {
            return Err(Failure::invalid_field_selection(path, name, ":calculation"));
        }
        let mut cast = FieldMap::new();
        for (key, value) in given.into_iter().flatten() {
            let key = snake(key);
            let arg = calc.arguments.iter().find(|arg| arg.name == key).ok_or_else(|| Failure::invalid_calculation_args(path, name))?;
            cast.insert(key, value_input(arg.ty, value).map_err(|_| Failure::invalid_calculation_args(path, name))?);
        }
        if calc.arguments.iter().any(|arg| !arg.allow_nil && cast.get(arg.name).is_none_or(|v| v.is_null())) {
            return Err(Failure::invalid_calculation_args(path, name));
        }
        rules.loads.check(path, name)?;
        self.fields.push(name.to_string());
        self.calculation_args.insert(name.to_string(), cast);
        Ok(())
    }

    fn relationship(&mut self, name: &str, dest: &'static ResourceDef, selection: Selection, mut query: RelatedQuery, page: Option<NestedPage>) {
        query.select = Some(selection.attributes(dest));
        query.aggregates = selection.of(|field| dest.aggregate(field).is_some());
        query.calculations = selection.of(|field| dest.calculation(field).is_some());
        query.calculation_args = selection.calculation_args.clone();
        self.relationships.push((name.to_string(), Nested { selection, query, page }));
    }

    pub(crate) fn of(&self, matches: impl Fn(&str) -> bool) -> Vec<String> {
        self.fields.iter().filter(|name| matches(name)).cloned().collect()
    }

    /// The attributes a read must select: those chosen, the primary key, those the field
    /// policies check (redacting the record reads them), and each chosen relationship's
    /// key on this side.
    pub(crate) fn attributes(&self, resource: &ResourceDef) -> Vec<String> {
        let mut attrs: Vec<String> = self.of(|name| resource.attribute(name).is_some());
        let mut keep = |name: &str| {
            if resource.attribute(name).is_some() && !attrs.iter().any(|a| a == name) {
                attrs.push(name.to_string());
            }
        };
        if let Some(pk) = resource.primary_key() {
            keep(pk.name);
        }
        for field in ash_core::field_policy_fields(resource) {
            keep(field);
        }
        for (rel_name, _) in &self.relationships {
            if let Some(rel) = resource.relationship(rel_name) {
                for column in rel.source_columns() {
                    keep(column);
                }
            }
        }
        attrs
    }

    /// This selection, reading `fields` too: what a keyset needs of each record.
    pub(crate) fn keeping<'a>(&self, fields: impl Iterator<Item = &'a str>) -> Selection {
        let mut selection = self.clone();
        for field in fields {
            if !selection.fields.iter().any(|f| f == field) {
                selection.fields.push(field.to_string());
            }
        }
        selection
    }
}

/// The path to a field's own fields.
fn child(path: &[String], name: &str) -> Vec<String> {
    let mut path = path.to_vec();
    path.push(name.to_string());
    path
}

/// A sort as text, as the generated client sends it (an array joined by commas).
pub(crate) fn sort_text(sort: &Json) -> String {
    match sort {
        Json::Array(items) => items.iter().filter_map(Json::as_str).collect::<Vec<_>>().join(","),
        Json::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn failure(err: &ash_core::Error) -> Failure {
    Failure::from_error(err).unwrap_or_else(|| Failure::new("invalid", "Invalid", err.to_string()))
}

/// A selection naming a field twice fails, whichever way it's named.
fn check_duplicates(items: &[Json], path: &[String]) -> Result<(), Failure> {
    let mut seen: Vec<String> = Vec::new();
    for item in items {
        let names: Vec<String> = match item {
            Json::String(name) => vec![snake(name)],
            Json::Object(map) => map.keys().map(|key| snake(key)).collect(),
            other => return Err(Failure::invalid_field_item(path, other)),
        };
        for name in names {
            if seen.contains(&name) {
                return Err(Failure::duplicate_field(path, &name));
            }
            seen.push(name);
        }
    }
    Ok(())
}

/// A relationship's `page`: a map of the keys its read's pagination takes.
fn nested_page(page: &Json, pagination: ash_core::Pagination, path: &[String], name: &str) -> Result<NestedPage, Failure> {
    let Json::Object(map) = page else {
        return Err(Failure::invalid_nested_page(path, name, ":not_a_map"));
    };
    let allowed: &[&str] = match (pagination.offset, pagination.keyset) {
        (true, false) => &["limit", "offset", "count"],
        (false, true) => &["limit", "after", "before", "count"],
        _ => &["limit", "offset", "count", "after", "before"],
    };
    let unknown: Vec<String> = map.keys().map(|key| snake(key)).filter(|key| !allowed.contains(&key.as_str())).collect();
    if !unknown.is_empty() {
        // As Elixir inspects them: a page key it knows an atom, any other a string.
        let keys: Vec<String> = unknown
            .iter()
            .map(|key| if ["limit", "offset", "count", "after", "before"].contains(&key.as_str()) { format!(":{key}") } else { format!("{key:?}") })
            .collect();
        return Err(Failure::invalid_nested_page(path, name, &format!("{{:unknown_keys, [{}]}}", keys.join(", "))));
    }
    let number = |key: &str| map.get(key).and_then(Json::as_u64).map(|n| n as usize);
    let text = |key: &str| map.get(key).and_then(Json::as_str).map(str::to_string);
    let offset = number("offset");
    let (after, before) = (text("after"), text("before"));
    Ok(NestedPage {
        limit: number("limit").or(pagination.default_limit),
        offset,
        keyset: offset.is_none() && (after.is_some() || before.is_some() || (pagination.keyset && !pagination.offset)),
        after,
        before,
        count: map.get("count").and_then(Json::as_bool) == Some(true),
    })
}
