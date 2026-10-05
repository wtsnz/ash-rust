//! ash-rust's GraphQL schema for the helpdesk matches the one AshGraphql generates
//! for the same domain in Elixir (`examples/elixir/astro-helpdesk/priv/schema.graphql`): the
//! same types, fields, arguments, input fields and enum values, with the same types and
//! nullability. Descriptions and ordering aside, a client written against one works
//! against the other.

use std::collections::BTreeMap;
use std::path::Path;

use async_graphql::parser::types::{TypeKind, TypeSystemDefinition};

/// One schema, flattened: `Type` → kind, and `Type.field` / `Type.field(arg)` / `Enum.VALUE`
/// → its type.
fn flatten(sdl: &str) -> BTreeMap<String, String> {
    let document = async_graphql::parser::parse_schema(sdl).expect("a schema");
    let mut out = BTreeMap::new();
    for definition in document.definitions {
        let TypeSystemDefinition::Type(definition) = definition else {
            continue;
        };
        let definition = definition.node;
        let name = definition.name.node.to_string();
        match definition.kind {
            TypeKind::Scalar => {
                out.insert(name, "scalar".into());
            }
            TypeKind::Object(object) => {
                out.insert(name.clone(), "type".into());
                for field in object.fields {
                    let field = field.node;
                    let key = format!("{name}.{}", field.name.node);
                    out.insert(key.clone(), field.ty.node.to_string());
                    for argument in field.arguments {
                        let argument = argument.node;
                        out.insert(format!("{key}({})", argument.name.node), argument.ty.node.to_string());
                    }
                }
            }
            TypeKind::InputObject(input) => {
                out.insert(name.clone(), "input".into());
                for field in input.fields {
                    let field = field.node;
                    out.insert(format!("{name}.{}", field.name.node), field.ty.node.to_string());
                }
            }
            TypeKind::Enum(values) => {
                out.insert(name.clone(), "enum".into());
                for value in values.values {
                    out.insert(format!("{name}.{}", value.node.value.node), "value".into());
                }
            }
            TypeKind::Interface(_) => {
                out.insert(name, "interface".into());
            }
            TypeKind::Union(_) => {
                out.insert(name, "union".into());
            }
        }
    }
    out
}

/// Differences that only widen the schema, so a client of AshGraphql's still works.
///
/// ash-rust's text filters take `like` and `ilike` on any data layer; AshGraphql offers them
/// only where the data layer defines them (AshPostgres does, ETS doesn't), and the Elixir
/// desk runs on ETS.
fn widens(key: &str) -> bool {
    key.split('.').next().is_some_and(|ty| ty.contains("Filter"))
        && [".like", ".ilike"].iter().any(|op| key.ends_with(op))
}

/// Built-in types either schema may print.
fn builtin(key: &str) -> bool {
    let name = key.split(['.', '(']).next().unwrap_or(key);
    name.starts_with("__") || ["String", "Int", "Float", "Boolean", "ID"].contains(&name)
}

#[test]
fn the_schema_matches_ash_graphql() {
    let reference_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../elixir/astro-helpdesk/priv/schema.graphql");
    let reference = flatten(&std::fs::read_to_string(reference_path).expect("the Elixir schema"));
    let schema = ash_graphql::AshGraphQL::from_domain(&astro_helpdesk::HELPDESK_DOMAIN)
        .with_pubsub(ash_pubsub::PubSub::new())
        .with_dataloader()
        .finish::<ash_memory::Memory>()
        .expect("the schema");
    let ours = flatten(&schema.sdl());

    let mut differences = Vec::new();
    for (key, expected) in &reference {
        if builtin(key) {
            continue;
        }
        match ours.get(key) {
            None => differences.push(format!("missing   {key}: {expected}")),
            Some(actual) if actual != expected => {
                differences.push(format!("differs   {key}: {actual}, AshGraphql has {expected}"));
            }
            Some(_) => {}
        }
    }
    for (key, actual) in &ours {
        if !builtin(key) && !widens(key) && !reference.contains_key(key) {
            differences.push(format!("extra     {key}: {actual}"));
        }
    }
    if !differences.is_empty() {
        let shown: Vec<&String> = differences.iter().collect();
        panic!(
            "{} differences from AshGraphql's schema:\n{}",
            differences.len(),
            shown.iter().map(|d| d.as_str()).collect::<Vec<_>>().join("\n")
        );
    }
}
