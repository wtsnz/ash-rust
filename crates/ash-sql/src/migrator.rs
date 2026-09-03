use std::fs;
use std::path::{Path, PathBuf};

use ash_core::{utc_now_iso8601, Error, Result};
use crate::dialect::SqlDialect;

/// Represents an on-disk migration script pair.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationFile {
    pub version: String,
    pub name: String,
    pub up_path: PathBuf,
    pub down_path: Option<PathBuf>,
}

/// Abstract executor interface for executing SQL migration statements and querying applied versions.
#[allow(async_fn_in_trait)]
pub trait MigrationExecutor: Send + Sync {
    /// Executes a raw multi-statement SQL script.
    async fn execute_script(&self, sql: &str) -> Result<()>;

    /// Fetches all applied migration versions from `_ash_schema_migrations`.
    async fn applied_versions(&self) -> Result<Vec<String>>;

    /// Records a newly applied migration version in `_ash_schema_migrations`.
    async fn record_migration(&self, version: &str, name: &str) -> Result<()>;

    /// Removes a rolled-back migration version from `_ash_schema_migrations`.
    async fn remove_migration(&self, version: &str) -> Result<()>;
}

/// Manages and executes database migrations from a migrations directory.
pub struct Migrator<D: SqlDialect> {
    pub dialect: D,
    pub migrations_dir: PathBuf,
}

impl<D: SqlDialect> Migrator<D> {
    pub fn new(dialect: D, migrations_dir: impl AsRef<Path>) -> Self {
        Self {
            dialect,
            migrations_dir: migrations_dir.as_ref().to_path_buf(),
        }
    }

    /// Discovers all available migration files in the migrations directory, sorted by timestamp version.
    pub fn discover_migrations(&self) -> Result<Vec<MigrationFile>> {
        if !self.migrations_dir.exists() {
            return Ok(Vec::new());
        }

        let mut migrations = Vec::new();
        let entries = fs::read_dir(&self.migrations_dir)
            .map_err(|e| Error::DataLayer(format!("Failed to read migrations directory: {e}")))?;

        let dialect_name = self.dialect.name();
        let up_suffix_dialect = format!(".{dialect_name}.up.sql");

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                let is_up = filename.ends_with(&up_suffix_dialect)
                    || (filename.ends_with(".up.sql") && !filename.contains(".sqlite.") && !filename.contains(".postgres."));

                if is_up {
                    // Filename format: <version>_<name>...
                    if let Some((version, rest)) = filename.split_once('_') {
                        let name = rest
                            .trim_end_matches(&up_suffix_dialect)
                            .trim_end_matches(".up.sql");

                        // Look for corresponding down file
                        let down_name_dialect = format!("{version}_{name}.{dialect_name}.down.sql");
                        let down_name_generic = format!("{version}_{name}.down.sql");

                        let down_path_dialect = self.migrations_dir.join(down_name_dialect);
                        let down_path_generic = self.migrations_dir.join(down_name_generic);

                        let down_path = if down_path_dialect.exists() {
                            Some(down_path_dialect)
                        } else if down_path_generic.exists() {
                            Some(down_path_generic)
                        } else {
                            None
                        };

                        migrations.push(MigrationFile {
                            version: version.to_string(),
                            name: name.to_string(),
                            up_path: path,
                            down_path,
                        });
                    }
                }
            }
        }

        migrations.sort_by(|a, b| a.version.cmp(&b.version));
        Ok(migrations)
    }

    /// Returns the list of unapplied migrations.
    pub async fn pending_migrations<E: MigrationExecutor>(
        &self,
        executor: &E,
    ) -> Result<Vec<MigrationFile>> {
        let applied = executor.applied_versions().await?;
        let all = self.discover_migrations()?;
        let pending = all
            .into_iter()
            .filter(|m| !applied.contains(&m.version))
            .collect();
        Ok(pending)
    }

    /// Runs all pending migrations against the given executor, returning the list of applied versions.
    pub async fn run<E: MigrationExecutor>(&self, executor: &E) -> Result<Vec<String>> {
        let pending = self.pending_migrations(executor).await?;
        let mut applied_versions = Vec::new();

        for migration in pending {
            let sql = fs::read_to_string(&migration.up_path)
                .map_err(|e| Error::DataLayer(format!("Failed to read migration {}: {e}", migration.up_path.display())))?;

            executor.execute_script(&sql).await?;
            executor.record_migration(&migration.version, &migration.name).await?;
            applied_versions.push(migration.version);
        }

        Ok(applied_versions)
    }

    /// Rolls back the latest applied migration, returning the rolled-back version if any.
    pub async fn rollback<E: MigrationExecutor>(&self, executor: &E) -> Result<Option<String>> {
        let mut applied = executor.applied_versions().await?;
        applied.sort();

        let Some(latest_version) = applied.last() else {
            return Ok(None);
        };

        let all = self.discover_migrations()?;
        let migration = all
            .into_iter()
            .find(|m| m.version == *latest_version)
            .ok_or_else(|| Error::DataLayer(format!("Migration script for version {latest_version} not found on disk")))?;

        let down_path = migration.down_path.ok_or_else(|| {
            Error::DataLayer(format!("No rollback script found for migration version {latest_version}"))
        })?;

        let sql = fs::read_to_string(&down_path)
            .map_err(|e| Error::DataLayer(format!("Failed to read down migration {}: {e}", down_path.display())))?;

        executor.execute_script(&sql).await?;
        executor.remove_migration(&migration.version).await?;

        Ok(Some(migration.version))
    }

    /// Rolls back applied migrations down to (and not including) the target version.
    pub async fn rollback_to<E: MigrationExecutor>(
        &self,
        executor: &E,
        target_version: &str,
    ) -> Result<Vec<String>> {
        let mut rolled_back = Vec::new();
        loop {
            let mut applied = executor.applied_versions().await?;
            applied.sort();
            match applied.last() {
                Some(latest) if latest.as_str() > target_version => {
                    if let Some(v) = self.rollback(executor).await? {
                        rolled_back.push(v);
                    } else {
                        break;
                    }
                }
                _ => break,
            }
        }
        Ok(rolled_back)
    }

    /// Returns SQL statement for creating the `_ash_schema_migrations` tracking table.
    pub fn create_tracking_table_sql(&self) -> &'static str {
        "CREATE TABLE IF NOT EXISTS _ash_schema_migrations (
  version VARCHAR(255) PRIMARY KEY,
  name VARCHAR(255) NOT NULL,
  applied_at VARCHAR(255) NOT NULL
);"
    }
}

/// An in-memory mock [`MigrationExecutor`] useful for unit and integration tests.
#[derive(Clone, Default, Debug)]
pub struct MemoryMigrationExecutor {
    pub applied: std::sync::Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
    pub executed_scripts: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl MemoryMigrationExecutor {
    pub fn new() -> Self {
        Self::default()
    }
}

impl MigrationExecutor for MemoryMigrationExecutor {
    async fn execute_script(&self, sql: &str) -> Result<()> {
        let mut scripts = self.executed_scripts.lock().unwrap();
        scripts.push(sql.to_string());
        Ok(())
    }

    async fn applied_versions(&self) -> Result<Vec<String>> {
        let applied = self.applied.lock().unwrap();
        Ok(applied.iter().map(|(v, _, _)| v.clone()).collect())
    }

    async fn record_migration(&self, version: &str, name: &str) -> Result<()> {
        let mut applied = self.applied.lock().unwrap();
        applied.push((version.to_string(), name.to_string(), utc_now_iso8601()));
        Ok(())
    }

    async fn remove_migration(&self, version: &str) -> Result<()> {
        let mut applied = self.applied.lock().unwrap();
        applied.retain(|(v, _, _)| v != version);
        Ok(())
    }
}
