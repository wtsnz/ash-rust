use crate::diff::{SchemaOperation, diff_snapshots, diff_snapshots_with_renames};
use crate::snapshot::{ColumnSnapshot, TableSnapshot};

/// A column disappeared from a table while another appeared, so the planner cannot tell a rename
/// from a drop plus an add without asking.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameQuestion {
    pub table: String,
    pub added: String,
    pub candidates: Vec<String>,
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

    if !unresolved.is_empty() {
        return SchemaPlan {
            operations: Vec::new(),
            targets,
            unresolved,
            deferred_drops,
            renames,
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
        operations.extend(diff_snapshots_with_renames(
            previous,
            Some(target),
            &renames,
        ));
    }
    for previous in old {
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
    }
}

pub fn reverse_plan(
    old: &[TableSnapshot],
    new: &[TableSnapshot],
    renames: &[(String, String, String)],
) -> Vec<SchemaOperation> {
    let mut operations = Vec::new();
    for previous in old {
        let next = new.iter().find(|table| table.table == previous.table);
        let inverted: Vec<(&str, &str)> = renames
            .iter()
            .filter(|(table, _, _)| table == &previous.table)
            .map(|(_, old, new)| (new.as_str(), old.as_str()))
            .collect();
        operations.extend(diff_snapshots_with_renames(next, Some(previous), &inverted));
    }
    for next in new.iter().rev() {
        if old.iter().all(|table| table.table != next.table) {
            operations.extend(diff_snapshots(Some(next), None));
        }
    }
    operations
}
