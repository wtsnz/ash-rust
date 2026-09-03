use std::fs;
use std::path::Path;

use ash_core::{OnDelete, RelKind, ResourceDef};
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
                default: None,
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

/// Represents a foreign key constraint in a schema snapshot.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReferenceSnapshot {
    pub name: String,
    pub column: String,
    pub target_table: String,
    pub target_column: String,
    pub on_delete: String,
}
