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
    pub dev: bool,
    pub squash_history: bool,
    pub applied_versions: Option<Vec<String>>,
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
            dev: false,
            squash_history: false,
            applied_versions: None,
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
    DevAndName,
    MissingAppliedVersions,
    RollbackBeforeSquash,
    HandWrittenMigration { path: PathBuf },
    RuntimeAlreadyRunning,
    RollbackDevFirst { versions: Vec<String> },
    AmbiguousRenames(Vec<RenameQuestion>),
    DuplicateIndexName { table: String, name: String },
    DuplicateCheckName { table: String, name: String },
    Usage(String),
    Io(std::io::Error),
    Snapshot(serde_json::Error),
    Plan(ash_core::Error),
}

impl fmt::Display for CodegenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingName => write!(f, "a migration name is required when writing changes"),
            Self::DevAndName => write!(f, "--dev does not take a migration name"),
            Self::MissingAppliedVersions => write!(
                f,
                "--squash requires applied migration versions from the database"
            ),
            Self::RollbackBeforeSquash => write!(
                f,
                "roll back until `_ash_schema_migrations` has no rows before squashing history"
            ),
            Self::HandWrittenMigration { path } => write!(
                f,
                "hand-written migration `{}` blocks history squash",
                path.display()
            ),
            Self::RuntimeAlreadyRunning => write!(
                f,
                "cannot load applied migration versions while a Tokio runtime is already running"
            ),
            Self::RollbackDevFirst { versions } => write!(
                f,
                "roll back dev migrations and delete their SQL files before writing a named migration ({})",
                versions.join(", ")
            ),
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
            Self::DuplicateIndexName { table, name } => write!(
                f,
                "index `{name}` on table `{table}` clashes with an identity name"
            ),
            Self::DuplicateCheckName { table, name } => write!(
                f,
                "check `{name}` on table `{table}` is defined more than once"
            ),
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
    if options.mode == Mode::Write && options.dev && options.name.is_some() {
        return Err(CodegenError::DevAndName);
    }

    if options.squash_history {
        match &options.applied_versions {
            None => return Err(CodegenError::MissingAppliedVersions),
            Some(versions) if !versions.is_empty() => {
                return Err(CodegenError::RollbackBeforeSquash);
            }
            Some(_) => {}
        }
    }

    let resources = persistable_resources(resources);
    for resource in &resources {
        let identity_names: std::collections::HashSet<&str> =
            resource.identities.iter().map(|i| i.name).collect();
        for index in resource.indexes {
            if identity_names.contains(index.name) {
                return Err(CodegenError::DuplicateIndexName {
                    table: resource.table_name().to_string(),
                    name: index.name.to_string(),
                });
            }
        }
        let mut check_names = std::collections::HashSet::new();
        for check in resource.checks {
            if !check_names.insert(check.name) {
                return Err(CodegenError::DuplicateCheckName {
                    table: resource.table_name().to_string(),
                    name: check.name.to_string(),
                });
            }
        }
    }
    let new: Vec<TableSnapshot> = resources
        .iter()
        .map(|resource| TableSnapshot::from_resource(resource, dialect))
        .collect();

    let committed_dir = committed_snap_dir(options);
    let old = if options.squash_history {
        Vec::new()
    } else if options.dev {
        load_effective_old(options)?
    } else {
        load_snapshots(&committed_dir)?
    };

    let plan = plan_schema(&old, &new, resolver, options.drop_columns);
    if !plan.unresolved.is_empty() {
        return Err(CodegenError::AmbiguousRenames(plan.unresolved));
    }

    if options.mode == Mode::Write && !options.dev {
        let versions = dev_migration_versions(options)?;
        if !versions.is_empty() {
            return Err(CodegenError::RollbackDevFirst { versions });
        }
    }

    if options.squash_history
        && let Some(path) = hand_written_migration_path(options)?
    {
        return Err(CodegenError::HandWrittenMigration { path });
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
        Mode::Write if options.dev => write_plan(
            dialect,
            options,
            "dev",
            &dev_snap_dir(options),
            &plan,
            &up_sql,
            &down_sql,
        ),
        Mode::Write => {
            let name = options.name.as_deref().ok_or(CodegenError::MissingName)?;
            let outcome = write_plan(
                dialect,
                options,
                name,
                &committed_dir,
                &plan,
                &up_sql,
                &down_sql,
            )?;
            if options.squash_history
                && let CodegenOutcome::Written(migration) = &outcome
            {
                delete_other_generated_migrations(
                    options,
                    &migration.up_path,
                    &migration.down_path,
                )?;
            }
            delete_dev_snapshots(options)?;
            Ok(outcome)
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
    snap_dir: &Path,
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

    std::fs::create_dir_all(snap_dir)?;
    for entry in std::fs::read_dir(snap_dir)? {
        let path = entry?.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            std::fs::remove_file(path)?;
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

fn committed_snap_dir(options: &CodegenOptions) -> PathBuf {
    options.snapshots_dir.join(options.dialect.name())
}

fn dev_snap_dir(options: &CodegenOptions) -> PathBuf {
    committed_snap_dir(options).join("dev")
}

fn load_effective_old(options: &CodegenOptions) -> Result<Vec<TableSnapshot>, CodegenError> {
    let dev_dir = dev_snap_dir(options);
    let dev = load_snapshots(&dev_dir)?;
    if !dev.is_empty() {
        Ok(dev)
    } else {
        load_snapshots(&committed_snap_dir(options))
    }
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

fn dev_migration_versions(options: &CodegenOptions) -> Result<Vec<String>, CodegenError> {
    let mut versions = Vec::new();
    if !options.migrations_dir.exists() {
        return Ok(versions);
    }
    let dialect = options.dialect.name();
    let up_suffix = format!(".{dialect}.up.sql");
    for entry in std::fs::read_dir(&options.migrations_dir)? {
        let path = entry?.path();
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !filename.ends_with(&up_suffix) {
            continue;
        }
        let Some((version, rest)) = filename.split_once('_') else {
            continue;
        };
        let name = rest.trim_end_matches(&up_suffix);
        if name == "dev" {
            versions.push(version.to_string());
        }
    }
    versions.sort();
    Ok(versions)
}

fn delete_dev_snapshots(options: &CodegenOptions) -> Result<(), CodegenError> {
    let dir = dev_snap_dir(options);
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

fn is_generated_migration_filename(filename: &str, dialect: &str) -> bool {
    let up_suffix = format!(".{dialect}.up.sql");
    let down_suffix = format!(".{dialect}.down.sql");
    let stem = if let Some(stem) = filename.strip_suffix(&up_suffix) {
        stem
    } else if let Some(stem) = filename.strip_suffix(&down_suffix) {
        stem
    } else {
        return false;
    };
    let Some((version, name)) = stem.split_once('_') else {
        return false;
    };
    !version.is_empty()
        && !name.is_empty()
        && version.chars().all(|c| c.is_ascii_digit())
}

fn hand_written_migration_path(options: &CodegenOptions) -> Result<Option<PathBuf>, CodegenError> {
    if !options.migrations_dir.exists() {
        return Ok(None);
    }
    let dialect = options.dialect.name();
    let mut entries: Vec<_> = std::fs::read_dir(&options.migrations_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    entries.sort();
    for path in entries {
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !filename.ends_with(".sql") {
            continue;
        }
        if !is_generated_migration_filename(filename, dialect) {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn delete_other_generated_migrations(
    options: &CodegenOptions,
    keep_up: &Path,
    keep_down: &Path,
) -> Result<(), CodegenError> {
    if !options.migrations_dir.exists() {
        return Ok(());
    }
    let dialect = options.dialect.name();
    for entry in std::fs::read_dir(&options.migrations_dir)? {
        let path = entry?.path();
        if path == keep_up || path == keep_down {
            continue;
        }
        let Some(filename) = path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if is_generated_migration_filename(filename, dialect) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Reads applied versions from `_ash_schema_migrations`. A missing tracking table is an empty list.
pub async fn applied_migration_versions(database_url: &str) -> Result<Vec<String>, CodegenError> {
    if database_url.starts_with("postgres://") || database_url.starts_with("postgresql://") {
        let pool = sqlx::PgPool::connect(database_url)
            .await
            .map_err(|error| CodegenError::Usage(error.to_string()))?;
        match sqlx::query_scalar::<_, String>(
            "SELECT version FROM _ash_schema_migrations ORDER BY version ASC",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(versions) => Ok(versions),
            Err(error) if postgres_undefined_table(&error) => Ok(Vec::new()),
            Err(error) => Err(CodegenError::Usage(error.to_string())),
        }
    } else {
        let pool = sqlx::SqlitePool::connect(database_url)
            .await
            .map_err(|error| CodegenError::Usage(error.to_string()))?;
        match sqlx::query_scalar::<_, String>(
            "SELECT version FROM _ash_schema_migrations ORDER BY version ASC",
        )
        .fetch_all(&pool)
        .await
        {
            Ok(versions) => Ok(versions),
            Err(error) if sqlite_missing_table(&error) => Ok(Vec::new()),
            Err(error) => Err(CodegenError::Usage(error.to_string())),
        }
    }
}

fn postgres_undefined_table(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(db) => db.code().as_deref() == Some("42P01"),
        _ => false,
    }
}

fn sqlite_missing_table(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Database(db) => db.message().contains("no such table"),
        _ => false,
    }
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
    #[arg(long)]
    dev: bool,
    #[arg(long)]
    squash: bool,
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
    let applied_versions = if cli.squash {
        Some(load_applied_versions_for_squash()?)
    } else {
        None
    };
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
        dev: cli.dev,
        squash_history: cli.squash,
        applied_versions,
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

fn load_applied_versions_for_squash() -> Result<Vec<String>, CodegenError> {
    if tokio::runtime::Handle::try_current().is_ok() {
        return Err(CodegenError::RuntimeAlreadyRunning);
    }
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://ash.db".to_string());
    let runtime = tokio::runtime::Runtime::new().map_err(CodegenError::Io)?;
    runtime.block_on(applied_migration_versions(&url))
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
