//! Zod validation schema generator for Ash resources and action inputs.

use crate::types::{atom_values, input_fields, to_camel_case, to_pascal_case};
use ash_core::{ActionDef, ActionKind, AttrType, ResourceDef, Validation};

/// Generate Zod schema for a single attribute.
pub fn generate_attr_zod(attr_ty: &AttrType, allow_nil: bool) -> String {
    let base = match attr_ty {
        AttrType::Uuid => "z.string().uuid()".to_string(),
        AttrType::String
        | AttrType::CiString
        | AttrType::Date
        | AttrType::Binary
        | AttrType::UtcDatetime { .. }
        | AttrType::Inet
        | AttrType::Decimal => "z.string()".to_string(),
        AttrType::Float => "z.number()".to_string(),
        AttrType::Vector { dimensions } => format!("z.array(z.number()).length({dimensions})"),
        AttrType::Integer => "z.number().int()".to_string(),
        AttrType::Boolean => "z.boolean()".to_string(),
        AttrType::Atom { one_of, name } => {
            if one_of.is_empty() {
                "z.string()".to_string()
            } else {
                let options = atom_values(one_of, *name)
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("z.enum([{options}])")
            }
        }
        AttrType::Map => "z.record(z.string(), z.unknown())".to_string(),
        AttrType::Array { of } => format!("z.array({})", generate_attr_zod(of, false)),
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

    for (field, ty, required) in input_fields(res, action) {
        let field_schema = build_field_zod_schema(action, field, &ty, required);
        out.push_str(&format!("  {}: {field_schema},\n", to_camel_case(field)));
    }

    out.push_str("});\n\n");
    if schema_name != alias_schema_name {
        out.push_str(&format!(
            "export const {alias_schema_name} = {schema_name};\n\n"
        ));
    }
    Some(out)
}

/// Build a Zod chain for a field taking action-level validations into account.
fn build_field_zod_schema(action: &ActionDef, field_name: &str, ty: &AttrType, required: bool) -> String {
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
        // GraphQL takes an enum type's values upper-cased, so validate those.
        let values: Vec<String> = if let AttrType::Atom { name, .. } = ty {
            atom_values(allowed, *name)
        } else {
            allowed.iter().map(|s| s.to_string()).collect()
        };
        let options = values
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("z.enum([{options}])")
    } else {
        match ty {
            AttrType::Uuid => "z.string().uuid()".to_string(),
            AttrType::String
            | AttrType::CiString
            | AttrType::Date
            | AttrType::Binary
            | AttrType::UtcDatetime { .. }
            | AttrType::Inet
            | AttrType::Decimal => {
                "z.string()".to_string()
            }
            AttrType::Float => "z.number()".to_string(),
            AttrType::Vector { dimensions } => {
                format!("z.array(z.number()).length({dimensions})")
            }
            AttrType::Integer => "z.number().int()".to_string(),
            AttrType::Boolean => "z.boolean()".to_string(),
            AttrType::Atom { one_of, name } => {
                if one_of.is_empty() {
                    "z.string()".to_string()
                } else {
                    let options = atom_values(one_of, *name)
                        .iter()
                        .map(|s| format!("\"{s}\""))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("z.enum([{options}])")
                }
            }
            AttrType::Map => "z.record(z.string(), z.unknown())".to_string(),
            AttrType::Array { of } => format!("z.array({})", generate_attr_zod(of, false)),
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

    // Optional as the mutation's input takes it, unless the action requires it be present.
    if !required && !is_present {
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
        out.push_str(&format!("  {}: {},\n", to_camel_case(attr.name), field_schema));
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
