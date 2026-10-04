//! TypeScript type definitions generator for Ash resources.

use ash_core::{ActionDef, ActionKind, AttrType, RelKind, ResourceDef};

/// Convert a snake_case identifier to PascalCase (e.g. `open_ticket` -> `OpenTicket`).
pub fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;
    for ch in s.chars() {
        if ch == '_' || ch == '-' {
            capitalize_next = true;
        } else if capitalize_next {
            result.push(ch.to_ascii_uppercase());
            capitalize_next = false;
        } else {
            result.push(ch);
        }
    }
    result
}

/// Convert a snake_case identifier to camelCase (e.g. `open_ticket` -> `openTicket`).
pub fn to_camel_case(s: &str) -> String {
    let pascal = to_pascal_case(s);
    let mut chars = pascal.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
    }
}

/// Atom variants as GraphQL enum values, which are upper-cased.
pub fn enum_values(one_of: &[&str]) -> Vec<String> {
    one_of.iter().map(|value| value.to_uppercase()).collect()
}

/// An atom's values as they cross the wire: a named enum type's as GraphQL enum values,
/// an unnamed atom's, which GraphQL takes as a string, as they're stored.
pub fn atom_values(one_of: &[&str], name: Option<&str>) -> Vec<String> {
    if name.is_some() {
        enum_values(one_of)
    } else {
        one_of.iter().map(|value| value.to_string()).collect()
    }
}


/// `call_sign` → `CALL_SIGN`, as GraphQL names sort fields.
pub fn to_upper_snake(s: &str) -> String {
    s.to_ascii_uppercase()
}

/// Map an Ash `AttrType` to its corresponding TypeScript type string: an enum type's
/// values as GraphQL spells them (upper-cased), an unnamed atom's as stored.
pub fn attr_type_to_ts(ty: &AttrType) -> String {
    match ty {
        AttrType::Uuid
        | AttrType::String
        | AttrType::CiString
        | AttrType::Date
        | AttrType::Binary
        | AttrType::UtcDatetime { .. }
        | AttrType::Inet
        | AttrType::Decimal => {
            "string".to_string()
        }
        AttrType::Float | AttrType::Integer => "number".to_string(),
        AttrType::Vector { .. } => "number[]".to_string(),
        AttrType::Boolean => "boolean".to_string(),
        AttrType::Atom { one_of, name } => {
            if one_of.is_empty() {
                "string".to_string()
            } else {
                atom_values(one_of, *name)
                    .iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(" | ")
            }
        }
        AttrType::Map => "Record<string, unknown>".to_string(),
        AttrType::Array { of } => format!("Array<{}>", attr_type_to_ts(of)),
        // Its fields, as GraphQL serves its object.
        AttrType::Embedded(_) | AttrType::TypedMap { .. } => {
            let fields: Vec<String> = ty
                .fields()
                .unwrap_or_default()
                .iter()
                .map(|field| {
                    let nil = if field.allow_nil { " | null" } else { "" };
                    format!("{}{}: {}{nil}", to_camel_case(field.name), if field.allow_nil { "?" } else { "" }, attr_type_to_ts(&field.ty))
                })
                .collect();
            format!("{{ {} }}", fields.join("; "))
        }
        // One member's object, as GraphQL serves the union.
        AttrType::Union { name, members } => members
            .iter()
            .map(|member| format!("{{ __typename: \"{name}{}\"; value: {} }}", to_pascal_case(member.name), attr_type_to_ts(&member.ty)))
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// The filter a field of this type takes, or `None` for types GraphQL doesn't filter on:
/// AshGraphql's operators for the value type, and the text operators on text.
pub fn attr_type_to_filter_type(ty: &AttrType) -> Option<String> {
    match ty {
        AttrType::String | AttrType::CiString => Some("AshTextFilter".to_string()),
        AttrType::Map | AttrType::Array { .. } | AttrType::Vector { .. } | AttrType::Binary => None,
        other => Some(format!("AshFilter<{}>", attr_type_to_ts(other))),
    }
}

/// Generate common shared TypeScript types: sort order, keyset pages and filters.
pub fn generate_common_types() -> String {
    r#"// Common Ash TypeScript Types
export type SortOrder = "asc" | "desc";

/** A keyset page of records, as AshGraphql returns a paginated read. */
export interface PaginatedResult<T> {
  results: T[];
  /** Records matching the query across all pages. */
  count?: number | null;
  startKeyset?: string | null;
  endKeyset?: string | null;
}

/** An offset page of records, as AshGraphql returns a read that pages by offset. */
export interface OffsetPage<T> {
  results: T[];
  /** Records matching the query across all pages. */
  count?: number | null;
  hasNextPage?: boolean;
  hasPreviousPage?: boolean;
  pageNumber?: number;
  lastPage?: number;
  limit?: number;
}

/** The operators AshGraphql filters a field by. */
export interface AshFilter<T> {
  isNil?: boolean;
  eq?: T | null;
  notEq?: T | null;
  in?: (T | null)[];
  lessThan?: T;
  greaterThan?: T;
  lessThanOrEqual?: T;
  greaterThanOrEqual?: T;
  isDistinctFrom?: T | null;
  isNotDistinctFrom?: T | null;
}

/** A text field's filter: AshFilter's operators and AshGraphql's text operators. */
export interface AshTextFilter extends AshFilter<string> {
  contains?: string;
  stringStartsWith?: string;
  stringEndsWith?: string;
  like?: string;
  ilike?: string;
}
"#
    .to_string()
}

/// Generate TypeScript interface for an Ash `ResourceDef`.
pub fn generate_resource_interface(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    out.push_str(&format!("export interface {name} {{\n"));

    // Attributes
    for attr in res.attributes {
        let ts_type = attr_type_to_ts(&attr.ty);
        if attr.allow_nil {
            out.push_str(&format!("  {}?: {} | null;\n", to_camel_case(attr.name), ts_type));
        } else {
            out.push_str(&format!("  {}: {};\n", to_camel_case(attr.name), ts_type));
        }
    }

    // Relationships (optional in the base model since they are loaded via `include`)
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        match rel.kind {
            RelKind::BelongsTo | RelKind::HasOne => {
                out.push_str(&format!("  {}?: {} | null;\n", to_camel_case(rel.name), dest_name));
            }
            RelKind::HasMany | RelKind::ManyToMany => {
                out.push_str(&format!("  {}?: {}[];\n", to_camel_case(rel.name), dest_name));
            }
        }
    }

    // Calculations
    for calc in res.calculations {
        let ts_type = attr_type_to_ts(&calc.ty);
        out.push_str(&format!("  {}?: {} | null;\n", to_camel_case(calc.name), ts_type));
    }

    // Aggregates
    for agg in res.aggregates {
        let ts_type = attr_type_to_ts(&agg.ty);
        out.push_str(&format!("  {}?: {} | null;\n", to_camel_case(agg.name), ts_type));
    }

    out.push_str("}\n\n");
    out
}

/// What a mutation's input holds, as ash-graphql builds it: the attributes the action
/// accepts and its arguments, each with whether it's required. A create requires an
/// accepted attribute that can't be nil and has no default; an update requires none.
pub fn input_fields(res: &ResourceDef, action: &ActionDef) -> Vec<(&'static str, AttrType, bool)> {
    let mut fields = Vec::new();
    if matches!(action.kind, ActionKind::Create | ActionKind::Update) {
        for attr in res.attributes {
            if action.accept.contains(&attr.name) {
                let required = action.kind == ActionKind::Create
                    && !attr.allow_nil
                    && attr.default_fn.is_none()
                    && !attr.generated;
                fields.push((attr.name, attr.ty, required));
            }
        }
    }
    for arg in action.arguments {
        fields.push((arg.name, arg.ty, !arg.allow_nil && arg.default.is_none()));
    }
    if matches!(action.kind, ActionKind::Update | ActionKind::Destroy)
        && let Some(version) = res.optimistic_lock_attribute()
    {
        fields.push((version, AttrType::Integer, false));
    }
    fields
}

/// Generate TypeScript action input interface (e.g. `OpenTicketInput`).
pub fn generate_action_input_interface(res: &ResourceDef, action_name: &str) -> Option<String> {
    let action = res.action(action_name)?;
    if action.kind == ActionKind::Read {
        return None;
    }

    let action_pascal = to_pascal_case(action.name);
    let input_name = format!("{action_pascal}{}Input", res.name);
    let alias_name = format!("{}{action_pascal}Input", res.name);
    let mut out = String::new();
    out.push_str(&format!("export interface {input_name} {{\n"));

    for (field, ty, required) in input_fields(res, action) {
        let ts_type = attr_type_to_ts(&ty);
        if required {
            out.push_str(&format!("  {}: {};\n", to_camel_case(field), ts_type));
        } else {
            out.push_str(&format!("  {}?: {} | null;\n", to_camel_case(field), ts_type));
        }
    }
    out.push_str("}\n\n");
    if input_name != alias_name {
        out.push_str(&format!("export type {alias_name} = {input_name};\n\n"));
    }
    Some(out)
}

/// Generate Resource Filter Input (e.g. `TicketFilterInput`), as AshGraphql's: each
/// attribute, aggregate and calculation, each relationship's filter, and `and` / `or` /
/// `not` lists.
pub fn generate_resource_filter_input(res: &ResourceDef) -> String {
    let name = res.name;
    let mut out = format!("export interface {name}FilterInput {{\n");
    let fields = res
        .attributes
        .iter()
        .map(|attr| (attr.name, attr.ty))
        .chain(res.aggregates.iter().map(|agg| (agg.name, agg.ty)))
        .chain(res.calculations.iter().map(|calc| (calc.name, calc.ty)));
    for (field, ty) in fields {
        if let Some(filter_type) = attr_type_to_filter_type(&ty) {
            out.push_str(&format!("  {}?: {filter_type};\n", to_camel_case(field)));
        }
    }
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        out.push_str(&format!("  {}?: {dest_name}FilterInput;\n", to_camel_case(rel.name)));
    }
    out.push_str(&format!("  and?: {name}FilterInput[];\n"));
    out.push_str(&format!("  or?: {name}FilterInput[];\n"));
    out.push_str(&format!("  not?: {name}FilterInput[];\n"));
    out.push_str("}\n\n");
    out
}

/// The fields a resource sorts by, as ash-graphql's sort enum has them: attributes,
/// aggregates, and calculations that take no arguments.
pub fn sort_fields(res: &ResourceDef) -> Vec<&'static str> {
    res.attributes
        .iter()
        .map(|a| a.name)
        .chain(res.aggregates.iter().map(|a| a.name))
        .chain(res.calculations.iter().filter(|c| c.arguments.is_empty()).map(|c| c.name))
        .collect()
}

/// Generate Resource Sort Input (e.g. `TicketSortInput`): fields in camelCase, which the
/// client turns into the schema's `TICKET_FIELD` enum values.
pub fn generate_resource_sort_input(res: &ResourceDef) -> String {
    let name = res.name;
    let fields = sort_fields(res)
        .into_iter()
        .map(|field| format!("\"{}\"", to_camel_case(field)))
        .collect::<Vec<_>>()
        .join(" | ");
    format!(
        "export type {name}SortField = {fields};\n\nexport interface {name}SortInput {{\n  field: {name}SortField;\n  order?: SortOrder;\n}}\n\n"
    )
}

/// Generate Resource Include Input (e.g. `TicketInclude`).
pub fn generate_resource_include_input(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    out.push_str(&format!("export interface {name}Include {{\n"));
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        out.push_str(&format!("  {}?: boolean | {dest_name}Include;\n", to_camel_case(rel.name)));
    }
    out.push_str("}\n\n");
    out
}

/// Generate all type definitions for a given ResourceDef.
pub fn generate_resource_types(res: &ResourceDef) -> String {
    let mut out = String::new();

    // 1. Model interface
    out.push_str(&generate_resource_interface(res));

    // 2. Action input interfaces
    for action in res.actions {
        if let Some(input_str) = generate_action_input_interface(res, action.name) {
            out.push_str(&input_str);
        }
    }

    // 3. Filter input
    out.push_str(&generate_resource_filter_input(res));

    // 4. Sort input
    out.push_str(&generate_resource_sort_input(res));

    // 5. Include input
    out.push_str(&generate_resource_include_input(res));

    out
}
