use std::fs;
use std::path::Path;

use ash_core::{OnDelete, RelKind, ResourceDef, Value};
use serde::{Deserialize, Serialize};

use crate::dialect::SqlDialect;

/// Represents a serializable snapshot of a table's database schema.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TableSnapshot {
    pub format_version: u32,
    pub table: String,
    pub schema: Option<String>,
    pub columns: Vec<ColumnSnapshot>,
    pub primary_key: Vec<String>,
    pub identities: Vec<IdentitySnapshot>,
    #[serde(default)]
    pub indexes: Vec<IndexSnapshot>,
    pub references: Vec<ReferenceSnapshot>,
}

impl TableSnapshot {
    /// Builds a [`TableSnapshot`] from an Ash [`ResourceDef`] and concrete [`SqlDialect`].
    pub fn from_resource<D: SqlDialect>(resource: &ResourceDef, dialect: &D) -> Self {
        let mut columns = Vec::new();
        let mut primary_key = Vec::new();

        for attr in resource.attributes {
            if attr.primary_key {
                primary_key.push(attr.name.to_string());
            }
            columns.push(ColumnSnapshot {
                name: attr.name.to_string(),
                sql_type: dialect.column_type(attr),
                nullable: attr.allow_nil && !attr.primary_key,
                default: attribute_sql_default(attr, dialect),
                is_primary_key: attr.primary_key,
            });
        }

        let mut identities = Vec::new();
        for id in resource.identities {
            identities.push(IdentitySnapshot {
                name: format!("idx_{}_{}", resource.table_name(), id.name),
                columns: id.keys.iter().map(|k| k.to_string()).collect(),
                unique: true,
            });
        }

        let mut indexes = Vec::new();
        for index in resource.indexes {
            indexes.push(IndexSnapshot {
                name: format!("idx_{}_{}", resource.table_name(), index.name),
                columns: index.keys.iter().map(|k| k.to_string()).collect(),
            });
        }

        let mut references = Vec::new();
        for rel in resource.relationships {
            if rel.kind == RelKind::BelongsTo {
                let dest = (rel.destination)();
                let on_delete_str = match rel.on_delete {
                    OnDelete::Cascade => "CASCADE",
                    OnDelete::Nilify => "SET NULL",
                    OnDelete::Restrict => "RESTRICT",
                    OnDelete::Nothing => "NO ACTION",
                };
                references.push(ReferenceSnapshot {
                    name: format!("fk_{}_{}", resource.table_name(), rel.name),
                    column: rel.source_attribute.to_string(),
                    target_table: dest.table_name().to_string(),
                    target_column: rel.destination_attribute.to_string(),
                    on_delete: on_delete_str.to_string(),
                });
            }
        }

        Self {
            format_version: 1,
            table: resource.table_name().to_string(),
            schema: None,
            columns,
            primary_key,
            identities,
            indexes,
            references,
        }
    }

    /// Saves the snapshot to a JSON file on disk, creating directories as needed.
    pub fn save_to_file(&self, path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Loads a snapshot from a JSON file on disk.
    pub fn load_from_file(path: impl AsRef<Path>) -> Result<Self, std::io::Error> {
        let content = fs::read_to_string(path)?;
        let snapshot = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        Ok(snapshot)
    }

    /// Loads a snapshot if the file exists, or returns `None`.
    pub fn load_opt(path: impl AsRef<Path>) -> Option<Self> {
        Self::load_from_file(path).ok()
    }
}

/// Represents a single column in a schema snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnSnapshot {
    pub name: String,
    pub sql_type: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub is_primary_key: bool,
}

/// Represents a unique constraint or index in a schema snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentitySnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

/// Represents a non-unique index in a schema snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexSnapshot {
    pub name: String,
    pub columns: Vec<String>,
}

/// Represents a foreign key constraint in a schema snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceSnapshot {
    pub name: String,
    pub column: String,
    pub target_table: String,
    pub target_column: String,
    pub on_delete: String,
}

/// SQL default for a constant attribute default. A function that returns a new value each call is skipped.
pub fn attribute_sql_default<D: SqlDialect>(
    attr: &ash_core::AttributeDef,
    dialect: &D,
) -> Option<String> {
    let default_fn = attr.default_fn?;
    let first = default_fn();
    let second = default_fn();
    if first != second {
        return None;
    }
    sql_literal(dialect, &first)
}

pub fn sql_literal<D: SqlDialect>(dialect: &D, value: &Value) -> Option<String> {
    match value {
        Value::Null => Some("NULL".to_string()),
        Value::Bool(v) => Some(dialect.boolean_literal(*v).to_string()),
        Value::Int(v) => Some(v.to_string()),
        Value::String(v) => Some(format!("'{}'", v.replace('\'', "''"))),
        Value::Uuid(v) => Some(format!("'{v}'")),
        Value::Map(_) | Value::Array(_) => None,
    }
}

/// Resources that persist as tables, parents before children.
pub fn persistable_resources<'a>(resources: &[&'a ResourceDef]) -> Vec<&'a ResourceDef> {
    let mut pending: Vec<&ResourceDef> = resources
        .iter()
        .copied()
        .filter(|resource| !resource.is_embedded())
        .collect();
    let names: std::collections::HashSet<&str> = pending.iter().map(|r| r.table_name()).collect();
    let mut ordered = Vec::new();
    while !pending.is_empty() {
        let next = pending.iter().position(|resource| {
            resource.relationships.iter().all(|rel| {
                if rel.kind != RelKind::BelongsTo {
                    return true;
                }
                let dest = (rel.destination)();
                !names.contains(dest.table_name())
                    || ordered
                        .iter()
                        .any(|done: &&ResourceDef| done.table_name() == dest.table_name())
            })
        });
        match next {
            Some(i) => ordered.push(pending.remove(i)),
            None => ordered.append(&mut pending),
        }
    }
    ordered
}
