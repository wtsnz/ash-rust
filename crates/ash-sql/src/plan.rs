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
    let mut deferred_drops = Vec::new();
    let mut renames_by_table: Vec<(String, Vec<(String, String)>)> = Vec::new();
    let mut targets = Vec::new();

    for incoming in new {
        let previous = old.iter().find(|table| table.table == incoming.table);
        let mut target = incoming.clone();
        if let Some(previous) = previous {
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

            let mut table_renames = Vec::new();
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
                        table_renames.push((old_name, added_col.name.clone()));
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

            if !table_renames.is_empty() {
                renames_by_table.push((incoming.table.clone(), table_renames));
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

    let removed_tables: Vec<&TableSnapshot> = old
        .iter()
        .filter(|table| new.iter().all(|incoming| incoming.table != table.table))
        .collect();
    let mut table_renames = Vec::new();
    let mut claimed_tables = Vec::new();
    for incoming in new {
        if old.iter().any(|table| table.table == incoming.table) {
            continue;
        }
        let candidates: Vec<String> = removed_tables
            .iter()
            .filter(|table| !claimed_tables.contains(&table.table))
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
                claimed_tables.push(old_name.clone());
                table_renames.push((old_name, incoming.table.clone()));
            }
            Resolution::NotRenamed => {}
            Resolution::Unresolved | Resolution::RenamedFrom(_) => {
                unresolved.push(question);
            }
        }
    }

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
        let previous = old.iter().find(|table| table.table == target.table);
        let renames = renames_by_table
            .iter()
            .find(|(table, _)| table == &target.table)
            .map(|(_, pairs)| {
                pairs
                    .iter()
                    .map(|(old, new)| (old.as_str(), new.as_str()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some((old_name, _)) = table_renames
            .iter()
            .find(|(_, new_name)| new_name == &target.table)
        {
            let Some(previous) = old.iter().find(|table| table.table == *old_name) else {
                continue;
            };
            operations.push(SchemaOperation::RenameTable {
                old_name: old_name.clone(),
                new_name: target.table.clone(),
            });
            let previous_as_new = rename_snapshot(previous, &target.table);
            operations.extend(diff_snapshots(Some(&previous_as_new), Some(target)));
            continue;
        }
        operations.extend(diff_snapshots_with_renames(
            previous,
            Some(target),
            &renames,
        ));
    }
    for previous in old {
        if claimed_tables.iter().any(|name| name == &previous.table) {
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

fn rename_snapshot(snapshot: &TableSnapshot, new_name: &str) -> TableSnapshot {
    let mut copy = snapshot.clone();
    let old_name = copy.table.clone();
    copy.table = new_name.to_string();
    let rewrite = |name: &mut String, old_prefix: &str, new_prefix: &str| {
        if let Some(rest) = name.strip_prefix(old_prefix) {
            *name = format!("{new_prefix}{rest}");
        }
    };
    for identity in &mut copy.identities {
        rewrite(
            &mut identity.name,
            &format!("idx_{old_name}_"),
            &format!("idx_{new_name}_"),
        );
    }
    for index in &mut copy.indexes {
        rewrite(
            &mut index.name,
            &format!("idx_{old_name}_"),
            &format!("idx_{new_name}_"),
        );
    }
    for check in &mut copy.checks {
        rewrite(
            &mut check.name,
            &format!("ck_{old_name}_"),
            &format!("ck_{new_name}_"),
        );
    }
    for reference in &mut copy.references {
        rewrite(
            &mut reference.name,
            &format!("fk_{old_name}_"),
            &format!("fk_{new_name}_"),
        );
    }
    copy
}

pub fn reverse_plan(
    old: &[TableSnapshot],
    new: &[TableSnapshot],
    renames: &[(String, String, String)],
    table_renames: &[(String, String)],
) -> Vec<SchemaOperation> {
    let mut operations = Vec::new();
    for (old_name, new_name) in table_renames {
        let Some(previous) = old.iter().find(|table| table.table == *old_name) else {
            continue;
        };
        let Some(next) = new.iter().find(|table| table.table == *new_name) else {
            continue;
        };
        let previous_as_new = rename_snapshot(previous, new_name);
        operations.extend(diff_snapshots(Some(next), Some(&previous_as_new)));
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
        let inverted: Vec<(&str, &str)> = renames
            .iter()
            .filter(|(table, _, _)| table == &previous.table)
            .map(|(_, old, new)| (new.as_str(), old.as_str()))
            .collect();
        operations.extend(diff_snapshots_with_renames(next, Some(previous), &inverted));
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
