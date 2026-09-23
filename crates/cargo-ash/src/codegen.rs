use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use ash_core::{DomainDef, ResourceDef};
use ash_sql::{
    PostgresDialect, SchemaPlan, SqlDialect, SqliteDialect, TableSnapshot, emit_sql,
    generate_migration_version, persistable_resources, plan_schema, reverse_plan,
};
use clap::Parser;

pub use ash_sql::{NonInteractive, RenameQuestion, RenameResolver, Resolution};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
        }
    }

    fn parse(name: &str) -> Self {
        match name.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Self::Postgres,
            _ => Self::Sqlite,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Write,
    DryRun,
    Check,
}

#[derive(Clone, Debug)]
pub struct CodegenOptions {
    pub name: Option<String>,
    pub dialect: Dialect,
    pub migrations_dir: PathBuf,
    pub snapshots_dir: PathBuf,
    pub mode: Mode,
    pub drop_columns: bool,
}

impl CodegenOptions {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            name: None,
            dialect,
            migrations_dir: PathBuf::from("migrations"),
            snapshots_dir: PathBuf::from("resource_snapshots"),
            mode: Mode::Write,
            drop_columns: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenMigration {
    pub version: String,
    pub up_path: PathBuf,
    pub down_path: PathBuf,
    pub up_sql: String,
    pub down_sql: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodegenOutcome {
    NoChanges,
    WouldWrite { up_sql: String, down_sql: String },
    OutOfDate { up_sql: String },
    Written(WrittenMigration),
}

impl CodegenOutcome {
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::OutOfDate { .. } => 1,
            Self::NoChanges | Self::WouldWrite { .. } | Self::Written(_) => 0,
        }
    }
}

#[derive(Debug)]
pub enum CodegenError {
    MissingName,
    AmbiguousRenames(Vec<RenameQuestion>),
    Usage(String),
    Io(std::io::Error),
    Snapshot(serde_json::Error),
    Plan(ash_core::Error),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingName => write!(f, "a migration name is required when writing changes"),
            Self::AmbiguousRenames(questions) => {
                write!(f, "rename required: ")?;
                for question in questions {
                    write!(
                        f,
                        "{}.{} could replace [{}]; ",
                        question.table,
                        question.added,
                        question.candidates.join(", ")
                    )?;
                }
                Ok(())
            }
            Self::Usage(message) => write!(f, "{message}"),
            Self::Io(error) => write!(f, "{error}"),
            Self::Snapshot(error) => write!(f, "{error}"),
            Self::Plan(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CodegenError {}

impl From<std::io::Error> for CodegenError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for CodegenError {
    fn from(error: serde_json::Error) -> Self {
        Self::Snapshot(error)
    }
}

/// Answers rename questions from `table.old=new` pairs and leaves every other question unresolved.
pub struct ExplicitRenames(pub Vec<(String, String, String)>);

impl RenameResolver for ExplicitRenames {
    fn resolve(&mut self, question: &RenameQuestion) -> Resolution {
        for (table, old, new) in &self.0 {
            if table == &question.table
                && new == &question.added
                && question.candidates.iter().any(|c| c == old)
            {
                return Resolution::RenamedFrom(old.clone());
            }
        }
        Resolution::Unresolved
    }
}

/// Asks on stdin for each ambiguous column.
pub struct Prompt;

impl RenameResolver for Prompt {
    fn resolve(&mut self, question: &RenameQuestion) -> Resolution {
        let _ = writeln!(
            io::stderr(),
            "Table `{}`: added `{}`. Removed: {}.",
            question.table,
            question.added,
            question.candidates.join(", ")
        );
        let _ = write!(io::stderr(), "Rename from (blank to add a new column): ");
        let _ = io::stderr().flush();
        let mut line = String::new();
        if io::stdin().read_line(&mut line).is_err() {
            return Resolution::Unresolved;
        }
        let answer = line.trim();
        if answer.is_empty() {
            Resolution::NotRenamed
        } else if question.candidates.iter().any(|c| c == answer) {
            Resolution::RenamedFrom(answer.to_string())
        } else {
            Resolution::Unresolved
        }
    }
}

pub fn run(
    resources: &[&'static ResourceDef],
    options: &CodegenOptions,
    resolver: &mut dyn RenameResolver,
) -> Result<CodegenOutcome, CodegenError> {
    match options.dialect {
        Dialect::Sqlite => run_with(&SqliteDialect, resources, options, resolver),
        Dialect::Postgres => run_with(&PostgresDialect, resources, options, resolver),
    }
}

fn run_with<D: SqlDialect>(
    dialect: &D,
    resources: &[&'static ResourceDef],
    options: &CodegenOptions,
    resolver: &mut dyn RenameResolver,
) -> Result<CodegenOutcome, CodegenError> {
    let resources = persistable_resources(resources);
    let new: Vec<TableSnapshot> = resources
        .iter()
        .map(|resource| TableSnapshot::from_resource(resource, dialect))
        .collect();
    let old = load_snapshots(&options.snapshots_dir.join(dialect.name()))?;
    let plan = plan_schema(&old, &new, resolver, options.drop_columns);
    if !plan.unresolved.is_empty() {
        return Err(CodegenError::AmbiguousRenames(plan.unresolved));
    }

    let up_sql = render(
        dialect,
        &plan.operations,
        &old,
        &plan.targets,
        &plan.deferred_drops,
    );
    if plan.operations.is_empty() {
        return Ok(CodegenOutcome::NoChanges);
    }

    let down_ops = reverse_plan(&old, &plan.targets, &plan.renames);
    let down_sql = emit_sql(dialect, &down_ops, &plan.targets, &old);

    match options.mode {
        Mode::Check => Ok(CodegenOutcome::OutOfDate { up_sql }),
        Mode::DryRun => Ok(CodegenOutcome::WouldWrite { up_sql, down_sql }),
        Mode::Write => {
            let name = options.name.as_deref().ok_or(CodegenError::MissingName)?;
            write_plan(dialect, options, name, &plan, &up_sql, &down_sql)
        }
    }
}

fn render<D: SqlDialect>(
    dialect: &D,
    operations: &[ash_sql::SchemaOperation],
    old: &[TableSnapshot],
    targets: &[TableSnapshot],
    deferred_drops: &[(String, String)],
) -> String {
    let mut sql = emit_sql(dialect, operations, old, targets);
    for (table, column) in deferred_drops {
        let table = dialect.quote_identifier(table);
        let column = dialect.quote_identifier(column);
        let comment = format!("-- ALTER TABLE {table} DROP COLUMN {column};");
        if sql.is_empty() {
            sql = comment;
        } else {
            sql.push_str("\n\n");
            sql.push_str(&comment);
        }
    }
    sql
}

fn write_plan<D: SqlDialect>(
    dialect: &D,
    options: &CodegenOptions,
    name: &str,
    plan: &SchemaPlan,
    up_sql: &str,
    down_sql: &str,
) -> Result<CodegenOutcome, CodegenError> {
    std::fs::create_dir_all(&options.migrations_dir)?;
    let version = next_version(&options.migrations_dir);
    let dialect_name = dialect.name();
    let up_filename = format!("{version}_{name}.{dialect_name}.up.sql");
    let down_filename = format!("{version}_{name}.{dialect_name}.down.sql");
    let up_path = options.migrations_dir.join(&up_filename);
    let down_path = options.migrations_dir.join(&down_filename);
    let up_body = format!(
        "-- Migration: {up_filename}\n-- Generated automatically by ash-rust.\n\n{up_sql}\n"
    );
    let down_body = format!(
        "-- Migration: {down_filename}\n-- Generated automatically by ash-rust.\n\n{down_sql}\n"
    );
    std::fs::write(&up_path, &up_body)?;
    std::fs::write(&down_path, &down_body)?;

    let snap_dir = options.snapshots_dir.join(dialect_name);
    if snap_dir.exists() {
        for entry in std::fs::read_dir(&snap_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                std::fs::remove_file(path)?;
            }
        }
    }
    for snapshot in &plan.targets {
        snapshot.save_to_file(snap_dir.join(format!("{}.json", snapshot.table)))?;
    }

    Ok(CodegenOutcome::Written(WrittenMigration {
        version,
        up_path,
        down_path,
        up_sql: up_body,
        down_sql: down_body,
    }))
}

fn load_snapshots(dir: &Path) -> Result<Vec<TableSnapshot>, CodegenError> {
    let mut snapshots = Vec::new();
    if !dir.exists() {
        return Ok(snapshots);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            snapshots.push(TableSnapshot::load_from_file(path)?);
        }
    }
    Ok(snapshots)
}

fn next_version(dir: &Path) -> String {
    let mut version = generate_migration_version();
    let names = list_names(dir);
    while names
        .iter()
        .any(|name| name.starts_with(&format!("{version}_")))
    {
        version = increment_version(&version);
    }
    version
}

fn increment_version(version: &str) -> String {
    match version.parse::<u128>() {
        Ok(n) => format!("{:014}", n + 1),
        Err(_) => format!("{version}1"),
    }
}

fn list_names(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

#[derive(Parser, Debug)]
#[command(name = "ash")]
struct CodegenCli {
    name: Option<String>,
    #[arg(long)]
    check: bool,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    drop_columns: bool,
    #[arg(long, default_value = "sqlite")]
    dialect: String,
    #[arg(long, default_value = "migrations")]
    migrations_dir: PathBuf,
    #[arg(long, default_value = "resource_snapshots")]
    snapshots_dir: PathBuf,
    #[arg(long = "rename")]
    renames: Vec<String>,
}

pub fn run_cli<I, S>(
    domains: &[&'static DomainDef],
    args: I,
) -> Result<CodegenOutcome, CodegenError>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString> + Clone,
{
    let mut argv: Vec<std::ffi::OsString> = args.into_iter().map(Into::into).collect();
    if argv.get(1).is_some_and(|s| s == "codegen") {
        argv.remove(1);
    }
    let cli = CodegenCli::try_parse_from(argv).map_err(|e| CodegenError::Usage(e.to_string()))?;
    let resources: Vec<&'static ResourceDef> = domains
        .iter()
        .flat_map(|d| d.resources.iter().copied())
        .collect();
    let options = CodegenOptions {
        name: cli.name,
        dialect: Dialect::parse(&cli.dialect),
        migrations_dir: cli.migrations_dir,
        snapshots_dir: cli.snapshots_dir,
        mode: if cli.check {
            Mode::Check
        } else if cli.dry_run {
            Mode::DryRun
        } else {
            Mode::Write
        },
        drop_columns: cli.drop_columns,
    };
    let mut parsed = Vec::new();
    for spec in &cli.renames {
        let (table_old, new) = spec
            .split_once('=')
            .ok_or_else(|| CodegenError::Usage(format!("expected table.column=new, got {spec}")))?;
        let (table, old) = table_old
            .split_once('.')
            .ok_or_else(|| CodegenError::Usage(format!("expected table.column=new, got {spec}")))?;
        parsed.push((table.to_string(), old.to_string(), new.to_string()));
    }
    if parsed.is_empty() {
        run(&resources, &options, &mut NonInteractive)
    } else {
        run(&resources, &options, &mut ExplicitRenames(parsed))
    }
}

pub fn main(domains: &[&'static DomainDef]) -> ExitCode {
    match run_cli(domains, std::env::args_os()) {
        Ok(outcome) => {
            match &outcome {
                CodegenOutcome::NoChanges => println!("schema is up to date"),
                CodegenOutcome::WouldWrite { up_sql, .. } => print!("{up_sql}"),
                CodegenOutcome::OutOfDate { .. } => eprintln!("schema has pending changes"),
                CodegenOutcome::Written(migration) => {
                    println!("{}", migration.up_path.display());
                    println!("{}", migration.down_path.display());
                }
            }
            ExitCode::from(outcome.exit_code())
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}
