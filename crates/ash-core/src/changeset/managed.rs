use crate::action::{ActionDef, Change, ManagedRelType};
use crate::resource::Resource;
use crate::value::{FieldMap, Value};

/// Specifications for mutating or synchronizing a child relationship within a changeset.
#[derive(Clone, Debug)]
pub struct ManagedRelationshipSpec {
    pub relationship: &'static str,
    pub rel_type: ManagedRelType,
    pub inputs: Vec<FieldMap>,
}

pub(crate) fn extract_managed_relationships<R: Resource>(
    action: &ActionDef,
    arguments: &FieldMap,
) -> Vec<ManagedRelationshipSpec> {
    let mut managed_relationships = Vec::new();
    for change in action.changes {
        if let Change::ManageRelationship { relationship, rel_type } = change
            && let Some(val) = arguments.get(*relationship)
        {
            let mut inputs = Vec::new();
            match val {
                Value::Array(items) => {
                    for it in items {
                        if let Value::Map(m) = it {
                            inputs.push(m.clone());
                        }
                    }
                }
                Value::Map(m) => {
                    inputs.push(m.clone());
                }
                _ => {}
            }
            managed_relationships.push(ManagedRelationshipSpec {
                relationship,
                rel_type: *rel_type,
                inputs,
            });
        }
    }
    for rel in R::DEF.relationships {
        if !managed_relationships.iter().any(|m| m.relationship == rel.name)
            && let Some(val) = arguments.get(rel.name)
        {
            let mut inputs = Vec::new();
            match val {
                Value::Array(items) => {
                    for it in items {
                        if let Value::Map(m) = it {
                            inputs.push(m.clone());
                        }
                    }
                }
                Value::Map(m) => {
                    inputs.push(m.clone());
                }
                _ => {}
            }
            if !inputs.is_empty() {
                managed_relationships.push(ManagedRelationshipSpec {
                    relationship: rel.name,
                    rel_type: crate::action::ManagedRelType::DirectControl,
                    inputs,
                });
            }
        }
    }
    managed_relationships
}
