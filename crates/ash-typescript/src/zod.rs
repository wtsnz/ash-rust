//! Zod validation schema generator for Ash resources and action inputs.

use ash_core::{ActionDef, ActionKind, AttrType, ResourceDef, Validation};
use crate::types::to_pascal_case;

/// Generate Zod schema for a single attribute.
pub fn generate_attr_zod(attr_ty: &AttrType, allow_nil: bool) -> String {
    let base = match attr_ty {
        AttrType::Uuid => "z.string().uuid()".to_string(),
        AttrType::String | AttrType::UtcDatetime | AttrType::Decimal => "z.string()".to_string(),
        AttrType::Integer => "z.number().int()".to_string(),
        AttrType::Boolean => "z.boolean()".to_string(),
        AttrType::Atom { one_of } => {
            if one_of.is_empty() {
                "z.string()".to_string()
            } else {
                let options = one_of
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("z.enum([{options}])")
            }
        }
        AttrType::Map => "z.record(z.string(), z.unknown())".to_string(),
        AttrType::Array => "z.array(z.unknown())".to_string(),
    };

    if allow_nil {
        format!("{base}.nullable().optional()")
    } else {
        base
    }
}

/// Generate a Zod schema for an action input, incorporating validations (StringLength, Numericality, OneOf, Present).
pub fn generate_action_zod_schema(res: &ResourceDef, action: &ActionDef) -> Option<String> {
    if action.kind == ActionKind::Read {
        return None;
    }

    let action_pascal = to_pascal_case(action.name);
    let schema_name = format!("{action_pascal}{}InputSchema", res.name);
    let alias_schema_name = format!("{}{action_pascal}InputSchema", res.name);

    let mut out = String::new();
    out.push_str(&format!("export const {schema_name} = z.object({{\n"));

    if action.kind == ActionKind::Destroy {
        out.push_str("  id: z.string().uuid(),\n");
    } else {
        if action.kind == ActionKind::Update {
            out.push_str("  id: z.string().uuid().optional(),\n");
        }

        // Collect accepted fields
        for attr_name in action.accept {
            if let Some(attr) = res.attribute(attr_name) {
                let field_schema = build_field_zod_schema(res, action, attr_name, &attr.ty, attr.allow_nil);
                out.push_str(&format!("  {attr_name}: {field_schema},\n"));
            }
        }

        // Collect action arguments
        for arg in action.arguments {
            let field_schema = build_field_zod_schema(res, action, arg.name, &arg.ty, arg.allow_nil);
            out.push_str(&format!("  {}: {},\n", arg.name, field_schema));
        }
    }

    out.push_str("});\n\n");
    if schema_name != alias_schema_name {
        out.push_str(&format!("export const {alias_schema_name} = {schema_name};\n\n"));
    }
    Some(out)
}

/// Build a Zod chain for a field taking action-level validations into account.
fn build_field_zod_schema(
    _res: &ResourceDef,
    action: &ActionDef,
    field_name: &str,
    ty: &AttrType,
    allow_nil: bool,
) -> String {
    // Check if field has a OneOf validation
    let mut one_of_allowed: Option<&[&str]> = None;
    let mut string_length: Option<(Option<usize>, Option<usize>)> = None;
    let mut numericality: Option<(Option<i64>, Option<i64>)> = None;
    let mut is_present = false;

    for v in action.validations {
        match v {
            Validation::Present { field } if *field == field_name => {
                is_present = true;
            }
            Validation::OneOf { field, allowed } if *field == field_name => {
                one_of_allowed = Some(allowed);
            }
            Validation::StringLength { field, min, max } if *field == field_name => {
                string_length = Some((*min, *max));
            }
            Validation::Numericality { field, min, max } if *field == field_name => {
                numericality = Some((*min, *max));
            }
            _ => {}
        }
    }

    let mut schema = if let Some(allowed) = one_of_allowed {
        let options = allowed
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("z.enum([{options}])")
    } else {
        match ty {
            AttrType::Uuid => "z.string().uuid()".to_string(),
            AttrType::String | AttrType::UtcDatetime | AttrType::Decimal => "z.string()".to_string(),
            AttrType::Integer => "z.number().int()".to_string(),
            AttrType::Boolean => "z.boolean()".to_string(),
            AttrType::Atom { one_of } => {
                if one_of.is_empty() {
                    "z.string()".to_string()
                } else {
                    let options = one_of
                        .iter()
                        .map(|s| format!("\"{s}\""))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("z.enum([{options}])")
                }
            }
            AttrType::Map => "z.record(z.string(), z.unknown())".to_string(),
            AttrType::Array => "z.array(z.unknown())".to_string(),
        }
    };

    // Apply StringLength
    if let Some((min, max)) = string_length {
        if let Some(min_len) = min {
            schema.push_str(&format!(".min({min_len})"));
        }
        if let Some(max_len) = max {
            schema.push_str(&format!(".max({max_len})"));
        }
    }

    // Apply Numericality
    if let Some((min, max)) = numericality {
        if let Some(min_val) = min {
            schema.push_str(&format!(".min({min_val})"));
        }
        if let Some(max_val) = max {
            schema.push_str(&format!(".max({max_val})"));
        }
    }

    // Check optionality:
    // If it's an update action, fields not explicitly marked Present are optional
    let is_optional = (allow_nil || action.kind == ActionKind::Update) && !is_present;
    if is_optional {
        schema.push_str(".nullable().optional()");
    }

    schema
}

/// Generate runtime Zod schema for an entire resource record (useful for response validation).
pub fn generate_resource_zod_schema(res: &ResourceDef) -> String {
    let schema_name = format!("{}Schema", res.name);
    let mut out = String::new();
    out.push_str(&format!("export const {schema_name} = z.object({{\n"));

    for attr in res.attributes {
        let field_schema = generate_attr_zod(&attr.ty, attr.allow_nil);
        out.push_str(&format!("  {}: {},\n", attr.name, field_schema));
    }

    out.push_str("});\n\n");
    out
}

/// Generate all Zod schemas for a resource.
pub fn generate_resource_zod(res: &ResourceDef) -> String {
    let mut out = String::new();

    // 1. Response schema
    out.push_str(&generate_resource_zod_schema(res));

    // 2. Action schemas
    for action in res.actions {
        if let Some(schema) = generate_action_zod_schema(res, action) {
            out.push_str(&schema);
        }
    }

    out
}
