use crate::diff::{SchemaOperation, diff_snapshots, diff_snapshots_with_renames};
use crate::snapshot::{ColumnSnapshot, TableSnapshot};

/// A column disappeared from a table while another appeared, so the planner cannot tell a rename
/// from a drop plus an add without asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameQuestion {
    pub table: String,
    pub added: String,
    pub candidates: Vec<String>,
    /// `added` is a new table and `candidates` are tables that disappeared.
    pub table_rename: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    RenamedFrom(String),
    NotRenamed,
    Unresolved,
}

pub trait RenameResolver {
    fn resolve(&mut self, question: &RenameQuestion) -> Resolution;
}

impl<F: FnMut(&RenameQuestion) -> Resolution> RenameResolver for F {
    fn resolve(&mut self, question: &RenameQuestion) -> Resolution {
        self(question)
    }
}

/// Answers every question with [`Resolution::Unresolved`], so any ambiguity fails the plan.
pub struct NonInteractive;

impl RenameResolver for NonInteractive {
    fn resolve(&mut self, _question: &RenameQuestion) -> Resolution {
        Resolution::Unresolved
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaPlan {
    pub operations: Vec<SchemaOperation>,
    pub targets: Vec<TableSnapshot>,
    pub unresolved: Vec<RenameQuestion>,
    pub deferred_drops: Vec<(String, String)>,
    pub renames: Vec<(String, String, String)>,
    /// `(old_table, new_table)` pairs the resolver confirmed.
    pub table_renames: Vec<(String, String)>,
}

pub fn plan_schema(
    old: &[TableSnapshot],
    new: &[TableSnapshot],
    resolver: &mut dyn RenameResolver,
    drop_columns: bool,
) -> SchemaPlan {
    let mut unresolved = Vec::new();

    // Table renames come first, so a renamed table's columns are compared with its old
    // snapshot rather than treated as all new.
    let removed_tables: Vec<&TableSnapshot> = old
        .iter()
        .filter(|table| new.iter().all(|incoming| incoming.table != table.table))
        .collect();
    let mut table_renames: Vec<(String, String)> = Vec::new();
    for incoming in new {
        if old.iter().any(|table| table.table == incoming.table) {
            continue;
        }
        let candidates: Vec<String> = removed_tables
            .iter()
            .filter(|table| table_renames.iter().all(|(old, _)| old != &table.table))
            .map(|table| table.table.clone())
            .collect();
        if candidates.is_empty() {
            continue;
        }
        let question = RenameQuestion {
            table: incoming.table.clone(),
            added: incoming.table.clone(),
            candidates,
            table_rename: true,
        };
        match resolver.resolve(&question) {
            Resolution::RenamedFrom(old_name) if question.candidates.contains(&old_name) => {
                table_renames.push((old_name, incoming.table.clone()));
            }
            Resolution::NotRenamed => {}
            Resolution::Unresolved | Resolution::RenamedFrom(_) => {
                unresolved.push(question);
            }
        }
    }
    let renamed_from = |table: &str| {
        table_renames
            .iter()
            .find(|(_, new_name)| new_name == table)
            .and_then(|(old_name, _)| old.iter().find(|t| &t.table == old_name))
    };

    let mut deferred_drops = Vec::new();
    let mut renames_by_table: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut targets = Vec::new();
    for incoming in new {
        let previous = old
            .iter()
            .find(|table| table.table == incoming.table)
            .cloned()
            .or_else(|| renamed_from(&incoming.table).map(|t| t.renamed(&incoming.table)));
        let mut target = incoming.clone();
        if let Some(previous) = &previous {
            let added: Vec<&ColumnSnapshot> = incoming
                .columns
                .iter()
                .filter(|column| previous.columns.iter().all(|old| old.name != column.name))
                .collect();
            let removed: Vec<&ColumnSnapshot> = previous
                .columns
                .iter()
                .filter(|column| incoming.columns.iter().all(|new| new.name != column.name))
                .collect();

            let mut column_renames = Vec::new();
            let mut claimed_old = Vec::new();
            for added_col in &added {
                if removed.is_empty() {
                    continue;
                }
                let question = RenameQuestion {
                    table: incoming.table.clone(),
                    added: added_col.name.clone(),
                    candidates: removed
                        .iter()
                        .filter(|column| !claimed_old.contains(&column.name))
                        .map(|column| column.name.clone())
                        .collect(),
                    table_rename: false,
                };
                if question.candidates.is_empty() {
                    continue;
                }
                match resolver.resolve(&question) {
                    Resolution::RenamedFrom(old_name)
                        if question.candidates.contains(&old_name) =>
                    {
                        claimed_old.push(old_name.clone());
                        column_renames.push((old_name, added_col.name.clone()));
                    }
                    Resolution::NotRenamed => {}
                    Resolution::Unresolved | Resolution::RenamedFrom(_) => {
                        unresolved.push(question);
                    }
                }
            }

            for removed_col in &removed {
                if claimed_old.contains(&removed_col.name) {
                    continue;
                }
                if drop_columns {
                    continue;
                }
                target.columns.push((*removed_col).clone());
                deferred_drops.push((incoming.table.clone(), removed_col.name.clone()));
            }

            if !column_renames.is_empty() {
                renames_by_table.push((incoming.table.clone(), column_renames));
            }
        }
        targets.push(target);
    }

    let renames: Vec<(String, String, String)> = renames_by_table
        .iter()
        .flat_map(|(table, pairs)| {
            pairs
                .iter()
                .map(|(old, new)| (table.clone(), old.clone(), new.clone()))
                .collect::<Vec<_>>()
        })
        .collect();

    if !unresolved.is_empty() {
        return SchemaPlan {
            operations: Vec::new(),
            targets,
            unresolved,
            deferred_drops,
            renames,
            table_renames,
        };
    }

    let mut operations = Vec::new();
    for target in &targets {
        let column_renames = renames_by_table
            .iter()
            .find(|(table, _)| table == &target.table)
            .map(|(_, pairs)| {
                pairs
                    .iter()
                    .map(|(old, new)| (old.as_str(), new.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(previous) = renamed_from(&target.table) {
            operations.push(SchemaOperation::RenameTable {
                old_name: previous.table.clone(),
                new_name: target.table.clone(),
            });
            operations.extend(rename_objects(previous, &target.table));
            let previous_as_new = previous.renamed(&target.table);
            operations.extend(diff_snapshots_with_renames(
                Some(&previous_as_new),
                Some(target),
                &column_renames,
            ));
            continue;
        }
        let previous = old.iter().find(|table| table.table == target.table);
        operations.extend(diff_snapshots_with_renames(
            previous,
            Some(target),
            &column_renames,
        ));
    }
    for previous in old {
        if table_renames.iter().any(|(name, _)| name == &previous.table) {
            continue;
        }
        if targets.iter().all(|target| target.table != previous.table) {
            operations.extend(diff_snapshots(Some(previous), None));
        }
    }

    SchemaPlan {
        operations,
        targets,
        unresolved,
        deferred_drops,
        renames,
        table_renames,
    }
}

/// Renames the indexes and constraints whose generated names include the table name, which
/// `ALTER TABLE ... RENAME TO` leaves behind.
fn rename_objects(previous: &TableSnapshot, new_name: &str) -> Vec<SchemaOperation> {
    let renamed = previous.renamed(new_name);
    let mut operations = Vec::new();
    let index_names = previous
        .identities
        .iter()
        .map(|i| &i.name)
        .zip(renamed.identities.iter().map(|i| &i.name))
        .chain(
            previous
                .indexes
                .iter()
                .map(|i| &i.name)
                .zip(renamed.indexes.iter().map(|i| &i.name)),
        );
    for (old_name, renamed_name) in index_names {
        if old_name != renamed_name {
            operations.push(SchemaOperation::RenameIndex {
                table: new_name.to_string(),
                old_name: old_name.clone(),
                new_name: renamed_name.clone(),
            });
        }
    }
    let constraint_names = previous
        .checks
        .iter()
        .map(|c| &c.name)
        .zip(renamed.checks.iter().map(|c| &c.name))
        .chain(
            previous
                .references
                .iter()
                .map(|r| &r.name)
                .zip(renamed.references.iter().map(|r| &r.name)),
        );
    for (old_name, renamed_name) in constraint_names {
        if old_name != renamed_name {
            operations.push(SchemaOperation::RenameConstraint {
                table: new_name.to_string(),
                old_name: old_name.clone(),
                new_name: renamed_name.clone(),
            });
        }
    }
    operations
}

/// Undoes [`rename_objects`] while the table still has its new name.
fn unrename_objects(previous: &TableSnapshot, new_name: &str) -> Vec<SchemaOperation> {
    rename_objects(previous, new_name)
        .into_iter()
        .rev()
        .map(|op| match op {
            SchemaOperation::RenameIndex {
                table,
                old_name,
                new_name,
            } => SchemaOperation::RenameIndex {
                table,
                old_name: new_name,
                new_name: old_name,
            },
            SchemaOperation::RenameConstraint {
                table,
                old_name,
                new_name,
            } => SchemaOperation::RenameConstraint {
                table,
                old_name: new_name,
                new_name: old_name,
            },
            other => other,
        })
        .collect()
}

pub fn reverse_plan(
    old: &[TableSnapshot],
    new: &[TableSnapshot],
    renames: &[(String, String, String)],
    table_renames: &[(String, String)],
) -> Vec<SchemaOperation> {
    let inverted_for = |table: &str| -> Vec<(&str, &str)> {
        renames
            .iter()
            .filter(|(t, _, _)| t == table)
            .map(|(_, old, new)| (new.as_str(), old.as_str()))
            .collect()
    };
    let mut operations = Vec::new();
    for (old_name, new_name) in table_renames {
        let Some(previous) = old.iter().find(|table| table.table == *old_name) else {
            continue;
        };
        let Some(next) = new.iter().find(|table| table.table == *new_name) else {
            continue;
        };
        let previous_as_new = previous.renamed(new_name);
        operations.extend(diff_snapshots_with_renames(
            Some(next),
            Some(&previous_as_new),
            &inverted_for(new_name),
        ));
        operations.extend(unrename_objects(previous, new_name));
        operations.push(SchemaOperation::RenameTable {
            old_name: new_name.clone(),
            new_name: old_name.clone(),
        });
    }
    for previous in old {
        if table_renames.iter().any(|(old_name, _)| old_name == &previous.table) {
            continue;
        }
        let next = new.iter().find(|table| table.table == previous.table);
        operations.extend(diff_snapshots_with_renames(
            next,
            Some(previous),
            &inverted_for(&previous.table),
        ));
    }
    for next in new.iter().rev() {
        if table_renames.iter().any(|(_, new_name)| new_name == &next.table) {
            continue;
        }
        if old.iter().all(|table| table.table != next.table) {
            operations.extend(diff_snapshots(Some(next), None));
        }
    }
    operations
}
