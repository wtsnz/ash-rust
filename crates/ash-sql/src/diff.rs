use serde::{Deserialize, Serialize};

use crate::snapshot::{
    CheckSnapshot, ColumnSnapshot, IdentitySnapshot, IndexSnapshot, ReferenceSnapshot,
    TableSnapshot,
};

/// Represents a single structural DDL change operation between two database states.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SchemaOperation {
    CreateTable(TableSnapshot),
    DropTable(String),
    AddColumn {
        table: String,
        column: ColumnSnapshot,
    },
    DropColumn {
        table: String,
        name: String,
    },
    RenameColumn {
        table: String,
        old_name: String,
        new_name: String,
    },
    AlterColumnType {
        table: String,
        column: String,
        old_type: String,
        new_type: String,
    },
    SetNullable {
        table: String,
        column: String,
        nullable: bool,
    },
    SetDefault {
        table: String,
        column: String,
        default: Option<String>,
    },
    CreateIdentity {
        table: String,
        identity: IdentitySnapshot,
    },
    DropIdentity {
        table: String,
        name: String,
    },
    CreateIndex {
        table: String,
        index: IndexSnapshot,
    },
    DropIndex {
        table: String,
        name: String,
    },
    AddCheck {
        table: String,
        check: CheckSnapshot,
    },
    DropCheck {
        table: String,
        name: String,
    },
    AddReference {
        table: String,
        reference: ReferenceSnapshot,
    },
    DropReference {
        table: String,
        name: String,
    },
}

/// Diffs two snapshots for the same table and computes the list of [`SchemaOperation`]s.
pub fn diff_snapshots(
    old: Option<&TableSnapshot>,
    new: Option<&TableSnapshot>,
) -> Vec<SchemaOperation> {
    diff_snapshots_with_renames(old, new, &[])
}

/// Diffs two snapshots with explicit column rename mappings.
pub fn diff_snapshots_with_renames(
    old: Option<&TableSnapshot>,
    new: Option<&TableSnapshot>,
    renames: &[(&str, &str)],
) -> Vec<SchemaOperation> {
    match (old, new) {
        (None, None) => Vec::new(),
        (None, Some(n)) => vec![SchemaOperation::CreateTable(n.clone())],
        (Some(o), None) => vec![SchemaOperation::DropTable(o.table.clone())],
        (Some(o), Some(n)) => {
            let mut ops = Vec::new();
            let table = n.table.clone();

            // 1. Check renames
            let mut renamed_old = Vec::new();
            let mut renamed_new = Vec::new();

            for &(old_col, new_col) in renames {
                if o.columns.iter().any(|c| c.name == old_col)
                    && n.columns.iter().any(|c| c.name == new_col)
                {
                    ops.push(SchemaOperation::RenameColumn {
                        table: table.clone(),
                        old_name: old_col.to_string(),
                        new_name: new_col.to_string(),
                    });
                    renamed_old.push(old_col);
                    renamed_new.push(new_col);
                }
            }

            // 2. Dropped columns
            for o_col in &o.columns {
                if renamed_old.contains(&o_col.name.as_str()) {
                    continue;
                }
                if !n.columns.iter().any(|c| c.name == o_col.name) {
                    ops.push(SchemaOperation::DropColumn {
                        table: table.clone(),
                        name: o_col.name.clone(),
                    });
                }
            }

            // 3. Added columns
            for n_col in &n.columns {
                if renamed_new.contains(&n_col.name.as_str()) {
                    continue;
                }
                if !o.columns.iter().any(|c| c.name == n_col.name) {
                    ops.push(SchemaOperation::AddColumn {
                        table: table.clone(),
                        column: n_col.clone(),
                    });
                }
            }

            // 4. Altered columns
            for n_col in &n.columns {
                let old_match = if let Some(&(old_name, _)) = renames
                    .iter()
                    .find(|(_, new_name)| *new_name == n_col.name.as_str())
                {
                    o.columns.iter().find(|c| c.name == old_name)
                } else {
                    o.columns.iter().find(|c| c.name == n_col.name)
                };

                if let Some(o_col) = old_match {
                    if o_col.sql_type != n_col.sql_type {
                        ops.push(SchemaOperation::AlterColumnType {
                            table: table.clone(),
                            column: n_col.name.clone(),
                            old_type: o_col.sql_type.clone(),
                            new_type: n_col.sql_type.clone(),
                        });
                    }
                    if o_col.nullable != n_col.nullable {
                        ops.push(SchemaOperation::SetNullable {
                            table: table.clone(),
                            column: n_col.name.clone(),
                            nullable: n_col.nullable,
                        });
                    }
                    if o_col.default != n_col.default {
                        ops.push(SchemaOperation::SetDefault {
                            table: table.clone(),
                            column: n_col.name.clone(),
                            default: n_col.default.clone(),
                        });
                    }
                }
            }

            // 5. Identities (unique constraints / indexes)
            for o_id in &o.identities {
                if !n.identities.iter().any(|i| i == o_id) {
                    ops.push(SchemaOperation::DropIdentity {
                        table: table.clone(),
                        name: o_id.name.clone(),
                    });
                }
            }
            for n_id in &n.identities {
                if !o.identities.iter().any(|i| i == n_id) {
                    ops.push(SchemaOperation::CreateIdentity {
                        table: table.clone(),
                        identity: n_id.clone(),
                    });
                }
            }

            // 6. Non-unique indexes
            for o_idx in &o.indexes {
                if !n.indexes.iter().any(|i| i == o_idx) {
                    ops.push(SchemaOperation::DropIndex {
                        table: table.clone(),
                        name: o_idx.name.clone(),
                    });
                }
            }
            for n_idx in &n.indexes {
                if !o.indexes.iter().any(|i| i == n_idx) {
                    ops.push(SchemaOperation::CreateIndex {
                        table: table.clone(),
                        index: n_idx.clone(),
                    });
                }
            }

            for o_ck in &o.checks {
                if !n.checks.iter().any(|c| c == o_ck) {
                    ops.push(SchemaOperation::DropCheck {
                        table: table.clone(),
                        name: o_ck.name.clone(),
                    });
                }
            }
            for n_ck in &n.checks {
                if !o.checks.iter().any(|c| c == n_ck) {
                    ops.push(SchemaOperation::AddCheck {
                        table: table.clone(),
                        check: n_ck.clone(),
                    });
                }
            }

            for o_ref in &o.references {
                if !n.references.iter().any(|r| r == o_ref) {
                    ops.push(SchemaOperation::DropReference {
                        table: table.clone(),
                        name: o_ref.name.clone(),
                    });
                }
            }
            for n_ref in &n.references {
                if !o.references.iter().any(|r| r == n_ref) {
                    ops.push(SchemaOperation::AddReference {
                        table: table.clone(),
                        reference: n_ref.clone(),
                    });
                }
            }

            ops
        }
    }
}

/// Diffs multiple table snapshots to produce an aggregate migration plan.
pub fn diff_tables(
    old_tables: &[TableSnapshot],
    new_tables: &[TableSnapshot],
) -> Vec<SchemaOperation> {
    let mut ops = Vec::new();

    // Check new or modified tables
    for n in new_tables {
        let old = old_tables.iter().find(|o| o.table == n.table);
        ops.extend(diff_snapshots(old, Some(n)));
    }

    // Check dropped tables
    for o in old_tables {
        if !new_tables.iter().any(|n| n.table == o.table) {
            ops.extend(diff_snapshots(Some(o), None));
        }
    }

    ops
}
