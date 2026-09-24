//! TypeScript type definitions generator for Ash resources.

use ash_core::{ActionKind, AttrType, RelKind, ResourceDef};

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

/// Map an Ash `AttrType` to its corresponding TypeScript type string.
pub fn attr_type_to_ts(ty: &AttrType) -> String {
    match ty {
        AttrType::Uuid | AttrType::String | AttrType::Date | AttrType::UtcDatetime | AttrType::Decimal => {
            "string".to_string()
        }
        AttrType::Float | AttrType::Integer => "number".to_string(),
        AttrType::Boolean => "boolean".to_string(),
        AttrType::Atom { one_of } => {
            if one_of.is_empty() {
                "string".to_string()
            } else {
                one_of
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        }
        AttrType::Map => "Record<string, unknown>".to_string(),
        AttrType::Array => "unknown[]".to_string(),
    }
}

/// Map an Ash `AttrType` to its corresponding Filter type name.
pub fn attr_type_to_filter_type(ty: &AttrType) -> &'static str {
    match ty {
        AttrType::Uuid => "UuidFilter",
        AttrType::String | AttrType::Date | AttrType::UtcDatetime | AttrType::Decimal => "StringFilter",
        AttrType::Float => "FloatFilter",
        AttrType::Integer => "IntFilter",
        AttrType::Boolean => "BooleanFilter",
        AttrType::Atom { .. } => "StringFilter",
        AttrType::Map | AttrType::Array => "JsonFilter",
    }
}

/// Generate common shared TypeScript types (Pagination, Filters, SortOrder).
pub fn generate_common_types() -> String {
    r#"// Common Ash TypeScript Types
export type SortOrder = "asc" | "desc";

export interface PageInfo {
  hasNextPage: boolean;
  hasPreviousPage: boolean;
  startCursor?: string | null;
  endCursor?: string | null;
}

export interface PaginatedResult<T> {
  results: T[];
  totalCount?: number;
  pageInfo?: PageInfo;
}

export interface UuidFilter {
  eq?: string;
  neq?: string;
  in?: string[];
  is_nil?: boolean;
}

export interface StringFilter {
  eq?: string;
  neq?: string;
  contains?: string;
  starts_with?: string;
  ends_with?: string;
  in?: string[];
  is_nil?: boolean;
}

export interface FloatFilter {
  eq?: number | null;
  ne?: number | null;
  gt?: number | null;
  gte?: number | null;
  lt?: number | null;
  lte?: number | null;
  isNil?: boolean | null;
}

export interface IntFilter {
  eq?: number;
  neq?: number;
  gt?: number;
  gte?: number;
  lt?: number;
  lte?: number;
  in?: number[];
  is_nil?: boolean;
}

export interface BooleanFilter {
  eq?: boolean;
  neq?: boolean;
  is_nil?: boolean;
}

export interface JsonFilter {
  is_nil?: boolean;
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
            out.push_str(&format!("  {}?: {} | null;\n", attr.name, ts_type));
        } else {
            out.push_str(&format!("  {}: {};\n", attr.name, ts_type));
        }
    }

    // Relationships (optional in the base model since they are loaded via `include`)
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        match rel.kind {
            RelKind::BelongsTo | RelKind::HasOne => {
                out.push_str(&format!("  {}?: {} | null;\n", rel.name, dest_name));
            }
            RelKind::HasMany | RelKind::ManyToMany => {
                out.push_str(&format!("  {}?: {}[];\n", rel.name, dest_name));
            }
        }
    }

    // Calculations
    for calc in res.calculations {
        let ts_type = attr_type_to_ts(&calc.ty);
        out.push_str(&format!("  {}?: {} | null;\n", calc.name, ts_type));
    }

    // Aggregates
    for agg in res.aggregates {
        let ts_type = attr_type_to_ts(&agg.ty);
        out.push_str(&format!("  {}?: {} | null;\n", agg.name, ts_type));
    }

    out.push_str("}\n\n");
    out
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

    if action.kind == ActionKind::Destroy {
        out.push_str("  id: string;\n");
    } else {
        if action.kind == ActionKind::Update {
            out.push_str("  id?: string;\n");
        }
        // Accepted attributes
        for attr_name in action.accept {
            if let Some(attr) = res.attribute(attr_name) {
                let ts_type = attr_type_to_ts(&attr.ty);
                let is_optional = attr.allow_nil || action.kind == ActionKind::Update;
                if is_optional {
                    out.push_str(&format!("  {}?: {} | null;\n", attr.name, ts_type));
                } else {
                    out.push_str(&format!("  {}: {};\n", attr.name, ts_type));
                }
            }
        }

        // Action arguments
        for arg in action.arguments {
            let ts_type = attr_type_to_ts(&arg.ty);
            if arg.allow_nil {
                out.push_str(&format!("  {}?: {} | null;\n", arg.name, ts_type));
            } else {
                out.push_str(&format!("  {}: {};\n", arg.name, ts_type));
            }
        }
    }

    out.push_str("}\n\n");
    if input_name != alias_name {
        out.push_str(&format!("export type {alias_name} = {input_name};\n\n"));
    }
    Some(out)
}

/// Generate Resource Filter Input (e.g. `TicketFilterInput`).
pub fn generate_resource_filter_input(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    out.push_str(&format!("export interface {name}FilterInput {{\n"));
    for attr in res.attributes {
        let filter_type = attr_type_to_filter_type(&attr.ty);
        out.push_str(&format!("  {}?: {};\n", attr.name, filter_type));
    }
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        out.push_str(&format!("  {}?: {dest_name}FilterInput;\n", rel.name));
    }
    out.push_str(&format!("  and?: {name}FilterInput[];\n"));
    out.push_str(&format!("  or?: {name}FilterInput[];\n"));
    out.push_str(&format!("  not?: {name}FilterInput;\n"));
    out.push_str("}\n\n");
    out
}

/// Generate Resource Sort Input (e.g. `TicketSortInput`).
pub fn generate_resource_sort_input(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    let fields = res
        .attributes
        .iter()
        .map(|a| format!("\"{}\"", a.name))
        .collect::<Vec<_>>()
        .join(" | ");

    out.push_str(&format!("export type {name}SortField = {fields};\n\n"));
    out.push_str(&format!("export interface {name}SortInput {{\n"));
    out.push_str(&format!("  field: {name}SortField;\n"));
    out.push_str("  order?: SortOrder;\n");
    out.push_str("}\n\n");
    out
}

/// Generate Resource Include Input (e.g. `TicketInclude`).
pub fn generate_resource_include_input(res: &ResourceDef) -> String {
    let mut out = String::new();
    let name = res.name;

    out.push_str(&format!("export interface {name}Include {{\n"));
    for rel in res.relationships {
        let dest_name = (rel.destination)().name;
        out.push_str(&format!("  {}?: boolean | {dest_name}Include;\n", rel.name));
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
