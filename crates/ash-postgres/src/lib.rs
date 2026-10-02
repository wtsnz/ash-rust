//! # `ash-postgres`
//!
//! PostgreSQL data layer implementation for `ash-rust` powered by `sqlx`.
//! Provides high-performance single-roundtrip writes via `RETURNING *`,
//! native error code translation, schema multitenancy (`search_path`),
//! and transaction savepoint management.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use ash_core::{
    AttrType, CompiledQuery, DataLayer, Error, FieldMap, ResourceDef, Result, SchemaSupport,
    TransactionSupport, Value,
};
use ash_sql::{
    CompiledSql, MigrationExecutor, Migrator, PostgresDialect, QueryCompiler, SqlDialect, SqlParam,
    TableSnapshot,
};
use sqlx::Row;
use sqlx::postgres::{PgConnectOptions, PgPool, PgPoolOptions, PgQueryResult, PgRow};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone, Debug)]
enum PostgresSource {
    Pool(PgPool),
    Tx(Arc<Mutex<sqlx::pool::PoolConnection<sqlx::Postgres>>>),
}

/// PostgreSQL data layer for Ash.
#[derive(Clone, Debug)]
pub struct Postgres {
    source: PostgresSource,
}

impl Postgres {
    /// Creates a [`Postgres`] instance from an existing [`PgPool`].
    pub fn new(pool: PgPool) -> Self {
        Self {
            source: PostgresSource::Pool(pool),
        }
    }

    /// Connects to PostgreSQL using a connection string.
    pub async fn connect(url: &str) -> Result<Self> {
        let options = PgConnectOptions::from_str(url).map_err(map_sqlx)?;
        Self::connect_with(options).await
    }

    /// Connects to PostgreSQL using custom [`PgConnectOptions`].
    pub async fn connect_with(options: PgConnectOptions) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;
        Ok(Self::new(pool))
    }

    /// Returns a reference to the underlying [`PgPool`], if not inside an active transaction.
    pub fn pool(&self) -> Option<&PgPool> {
        match &self.source {
            PostgresSource::Pool(p) => Some(p),
            PostgresSource::Tx(_) => None,
        }
    }

    async fn execute_compiled(&self, compiled: &CompiledSql) -> Result<PgQueryResult> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.execute(pool).await.map_err(map_sqlx)
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.execute(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_compiled_resource(
        &self,
        compiled: &CompiledSql,
        resource: &ResourceDef,
    ) -> Result<PgQueryResult> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .execute(pool)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .execute(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_one_resource(
        &self,
        compiled: &CompiledSql,
        resource: &ResourceDef,
    ) -> Result<PgRow> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_one(pool)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_one(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_all_resource(
        &self,
        compiled: &CompiledSql,
        resource: &ResourceDef,
    ) -> Result<Vec<PgRow>> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_all(pool)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_all(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_optional_resource(
        &self,
        compiled: &CompiledSql,
        resource: &ResourceDef,
    ) -> Result<Option<PgRow>> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_optional(pool)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .fetch_optional(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_all(&self, compiled: &CompiledSql) -> Result<Vec<PgRow>> {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.fetch_all(pool).await.map_err(map_sqlx)
            }
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.fetch_all(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_raw(&self, sql: &str) -> Result<PgQueryResult> {
        match &self.source {
            PostgresSource::Pool(pool) => sqlx::query(sql).execute(pool).await.map_err(map_sqlx),
            PostgresSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                sqlx::query(sql)
                    .execute(&mut **guard)
                    .await
                    .map_err(map_sqlx)
            }
        }
    }

    /// Installs table and index DDL for the given Ash resources.
    ///
    /// Runs in one transaction under the migration lock, so several processes installing
    /// at once (for example app instances starting together) neither collide on
    /// `CREATE TABLE IF NOT EXISTS` nor see a table before its indexes exist.
    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        self.transaction(|tx| {
            let tx = tx.clone();
            async move {
                tx.execute_raw(&format!("SELECT pg_advisory_xact_lock({MIGRATION_LOCK_KEY})"))
                    .await?;
                tx.install_unlocked(resources).await
            }
        })
        .await
    }

    /// Creates `tenant`'s schema if needed and installs its tables, as [`install`](Self::install)
    /// does for the default schema. As with AshPostgres's tenant migrations, only resources
    /// with context multitenancy get a table per tenant; the others stay shared. Schema
    /// names must match `[A-Za-z_][A-Za-z0-9_]*`. With migrations, use
    /// [`migrate_schemas`](Self::migrate_schemas) instead.
    pub async fn install_tenant(&self, tenant: &str, resources: &[&ResourceDef]) -> Result<()> {
        validate_schema_name(tenant)?;
        let schema = quote_schema_name(tenant);
        let tenant_resources: Vec<&ResourceDef> = resources
            .iter()
            .copied()
            .filter(|res| {
                res.multitenancy
                    .is_some_and(|mt| mt.strategy == ash_core::MultitenancyStrategy::Context)
            })
            .collect();
        self.transaction(|tx| {
            let tx = tx.clone();
            async move {
                tx.execute_raw(&format!("SELECT pg_advisory_xact_lock({MIGRATION_LOCK_KEY})"))
                    .await?;
                tx.execute_raw(&format!("CREATE SCHEMA IF NOT EXISTS {schema}")).await?;
                // New tables land in the first schema on the path, for this transaction
                // only; `public` keeps extension types such as `citext` visible.
                tx.execute_raw(&format!("SET LOCAL search_path TO {schema}, public"))
                    .await?;
                tx.install_unlocked(&tenant_resources).await
            }
        })
        .await
    }

    async fn install_unlocked(&self, resources: &[&ResourceDef]) -> Result<()> {
        let resources = ash_sql::persistable_resources(resources);
        let dialect = PostgresDialect;
        let compiler = QueryCompiler::new(&dialect);
        // Tables that refer to each other are created without the keys to tables not yet
        // made, and those keys added after: no order creates them with every key inline.
        let creates = resources
            .iter()
            .map(|res| ash_sql::SchemaOperation::CreateTable(TableSnapshot::from_resource(res, &dialect)))
            .collect();
        for operation in ash_sql::defer_forward_references(creates) {
            match operation {
                ash_sql::SchemaOperation::CreateTable(table) => {
                    let res = resources
                        .iter()
                        .find(|res| res.table_name() == table.table)
                        .expect("a table for each resource");
                    for extension in ash_sql::required_extensions(&dialect, res) {
                        self.execute_raw(&extension).await?;
                    }
                    self.execute_raw(&ash_sql::emit_create_table(&dialect, &table)).await?;
                    for idx_ddl in compiler.compile_create_indexes(res)? {
                        self.execute_raw(&idx_ddl).await?;
                    }
                }
                ash_sql::SchemaOperation::AddReference { table, reference } => {
                    // Installing again finds the key already there.
                    let alter = ash_sql::emit_add_reference(&dialect, &table, &reference);
                    let literal = |text: &str| format!("'{}'", text.replace('\'', "''"));
                    self.execute_raw(&format!(
                        "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = {} \
                         AND conrelid = to_regclass({})) THEN {alter} END IF; END $$",
                        literal(&reference.name),
                        literal(&dialect.quote_identifier(&table)),
                    ))
                    .await?;
                }
                _ => {}
            }
        }
        let has_statements = resources.iter().any(|res| {
            res.statements
                .iter()
                .any(|s| s.dialects.is_empty() || s.dialects.contains(&"postgres"))
        });
        if has_statements {
            self.execute_raw(ash_sql::install::CREATE_STATEMENTS_TABLE).await?;
        }
        for res in &resources {
            for statement in res.statements {
                if !statement.dialects.is_empty() && !statement.dialects.contains(&"postgres") {
                    continue;
                }
                let table = res.table_name();
                let (name, up) = (statement.name, statement.up);
                let changed = self
                    .execute_raw(&ash_sql::install::record_new_statement(table, name, up))
                    .await?
                    .rows_affected()
                    + self
                        .execute_raw(&ash_sql::install::record_changed_statement(table, name, up))
                        .await?
                        .rows_affected();
                if changed > 0
                    && let Err(err) = self.execute_raw(up).await
                {
                    let _ = self
                        .execute_raw(&ash_sql::install::forget_statement(table, name))
                        .await;
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    /// Run pending declarative migrations from a directory.
    pub async fn migrate(&self, migrations_dir: impl AsRef<Path>) -> Result<Vec<String>> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        migrate(pool, migrations_dir).await
    }

    /// Create each Postgres schema if needed, migrate the same history into it, then restore
    /// the caller's `search_path`. Schema names must match `[A-Za-z_][A-Za-z0-9_]*`.
    pub async fn migrate_schemas(
        &self,
        schemas: &[&str],
        migrations_dir: impl AsRef<Path>,
    ) -> Result<()> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let previous: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(pool)
            .await
            .map_err(map_sqlx)?;
        let dir = migrations_dir.as_ref();
        for name in schemas {
            validate_schema_name(name)?;
            let quoted = quote_schema_name(name);
            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {quoted}"))
                .execute(pool)
                .await
                .map_err(map_sqlx)?;
            // `public` keeps database-wide extension types such as `citext` visible.
            let search_path = format!("{name},public");
            let opts = (*pool.connect_options())
                .clone()
                .options([("search_path", search_path.as_str())]);
            let tenant_pool = PgPoolOptions::new()
                .max_connections(1)
                .connect_with(opts)
                .await
                .map_err(map_sqlx)?;
            let migrated = migrate(&tenant_pool, dir).await;
            tenant_pool.close().await;
            migrated?;
        }
        set_search_path(pool, &previous).await?;
        Ok(())
    }

    /// Rollback the latest applied migration.
    pub async fn rollback(&self, migrations_dir: impl AsRef<Path>) -> Result<Option<String>> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let migrator = Migrator::new(PostgresDialect, migrations_dir);
        let executor = PgMigrationExecutor::new(pool.clone(), migrator.create_tracking_table_sql());
        executor.init().await?;
        migrator.rollback(&executor).await
    }
}

impl MigrationExecutor for Postgres {
    async fn execute_script(&self, sql: &str) -> Result<()> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let executor = PgMigrationExecutor::new(pool.clone(), "");
        executor.execute_script(sql).await
    }

    async fn applied_versions(&self) -> Result<Vec<String>> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let migrator = Migrator::new(PostgresDialect, ".");
        let executor = PgMigrationExecutor::new(pool.clone(), migrator.create_tracking_table_sql());
        executor.init().await?;
        executor.applied_versions().await
    }

    async fn record_migration(&self, version: &str, name: &str) -> Result<()> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let executor = PgMigrationExecutor::new(pool.clone(), "");
        executor.record_migration(version, name).await
    }

    async fn remove_migration(&self, version: &str) -> Result<()> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let executor = PgMigrationExecutor::new(pool.clone(), "");
        executor.remove_migration(version).await
    }

    async fn apply_migration(&self, sql: &str, version: &str, name: &str) -> Result<bool> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let migrator = Migrator::new(PostgresDialect, ".");
        let executor = PgMigrationExecutor::new(pool.clone(), migrator.create_tracking_table_sql());
        executor.init().await?;
        executor.apply_migration(sql, version, name).await
    }

    async fn revert_migration(&self, sql: &str, version: &str) -> Result<bool> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let executor = PgMigrationExecutor::new(pool.clone(), "");
        executor.revert_migration(sql, version).await
    }
}

impl SchemaSupport for Postgres {
    async fn install_resources(&self, resources: &[&ResourceDef]) -> Result<()> {
        self.install(resources).await
    }
}

impl DataLayer for Postgres {
    async fn create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        _id: Uuid,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_insert(resource, &fields)?;

        // Postgres supports RETURNING * for single-roundtrip writes!
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource, &[], &[])
    }

    async fn update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_update(resource, id, &fields)?;

        // Postgres RETURNING * executes update and returns the new row
        let opt_row = self.fetch_optional_resource(&compiled, resource).await?;
        match opt_row {
            Some(row) => row_to_fields(&row, resource, &[], &[]),
            None => {
                // If 0 rows were updated, check optimistic lock or not found
                if resource.optimistic_lock_attribute().is_some() {
                    let pk = resource
                        .primary_key()
                        .ok_or(Error::NoPrimaryKey(resource.name))?;
                    // Whether the row is there at all, read in the same tenant.
                    let exists = CompiledQuery {
                        filter: Some(ash_core::Filter::eq(pk.name, Value::Uuid(id))),
                        tenant: tenant.map(str::to_string),
                        limit: Some(1),
                        ..CompiledQuery::default()
                    };
                    let check_compiled = QueryCompiler::new(&dialect).compile_select(resource, &exists)?;
                    let rows = self.fetch_all(&check_compiled).await?;
                    if rows.is_empty() {
                        Err(Error::NotFound)
                    } else {
                        Err(Error::StaleRecord {
                            resource: resource.name,
                            id,
                        })
                    }
                } else {
                    Err(Error::NotFound)
                }
            }
        }
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: Uuid) -> Result<()> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_delete(resource, id)?;
        let result = self.execute_compiled(&compiled).await?;
        if result.rows_affected() == 0 {
            return Err(Error::NotFound);
        }
        Ok(())
    }

    async fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> Result<Vec<FieldMap>> {
        // A context-tenant resource's tables are qualified by the tenant's schema, as
        // AshPostgres prefixes them, so the session's `search_path` is never changed.
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_select(resource, query)?;
        let rows = self.fetch_all(&compiled).await?;
        rows.iter()
            .map(|row| row_to_fields(row, resource, &query.calculations, &query.aggregates))
            .collect()
    }

    async fn upsert(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        _id: Uuid,
        fields: FieldMap,
        identity: &ash_core::IdentityDef,
        update_fields: &[String],
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_upsert(resource, &fields, identity, update_fields)?;

        // Single-roundtrip write with RETURNING *
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource, &[], &[])
    }

    async fn bulk_create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_bulk_insert(resource, &rows)?;
        let pg_rows = self.fetch_all_resource(&compiled, resource).await?;
        pg_rows
            .iter()
            .map(|r| row_to_fields(r, resource, &[], &[]))
            .collect()
    }

    async fn bulk_destroy(&self, resource: &ResourceDef, tenant: Option<&str>, ids: &[Uuid]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_bulk_delete(resource, ids)?;
        self.execute_compiled_resource(&compiled, resource).await?;
        Ok(())
    }
}

impl TransactionSupport for Postgres {
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        match &self.source {
            PostgresSource::Pool(pool) => {
                let conn = pool.acquire().await.map_err(map_sqlx)?;
                let conn = Arc::new(Mutex::new(conn));
                {
                    let mut guard = conn.lock().await;
                    sqlx::query("BEGIN")
                        .execute(&mut **guard)
                        .await
                        .map_err(map_sqlx)?;
                }
                let tx_pg = Postgres {
                    source: PostgresSource::Tx(Arc::clone(&conn)),
                };
                match f(&tx_pg).await {
                    Ok(val) => {
                        let mut guard = conn.lock().await;
                        sqlx::query("COMMIT")
                            .execute(&mut **guard)
                            .await
                            .map_err(map_sqlx)?;
                        Ok(val)
                    }
                    Err(err) => {
                        let mut guard = conn.lock().await;
                        let _ = sqlx::query("ROLLBACK").execute(&mut **guard).await;
                        Err(err)
                    }
                }
            }
            PostgresSource::Tx(conn) => {
                let sp_name = format!("sp_{}", Uuid::new_v4().simple());
                {
                    let mut guard = conn.lock().await;
                    sqlx::query(&format!("SAVEPOINT {sp_name}"))
                        .execute(&mut **guard)
                        .await
                        .map_err(map_sqlx)?;
                }
                match f(self).await {
                    Ok(val) => {
                        let mut guard = conn.lock().await;
                        sqlx::query(&format!("RELEASE SAVEPOINT {sp_name}"))
                            .execute(&mut **guard)
                            .await
                            .map_err(map_sqlx)?;
                        Ok(val)
                    }
                    Err(err) => {
                        let mut guard = conn.lock().await;
                        let _ = sqlx::query(&format!("ROLLBACK TO SAVEPOINT {sp_name}"))
                            .execute(&mut **guard)
                            .await;
                        Err(err)
                    }
                }
            }
        }
    }
}

/// Runs declarative migrations against a PostgreSQL connection pool.
pub async fn migrate(pool: &PgPool, migrations_dir: impl AsRef<Path>) -> Result<Vec<String>> {
    let migrator = Migrator::new(PostgresDialect, migrations_dir);
    let executor = PgMigrationExecutor::new(pool.clone(), migrator.create_tracking_table_sql());
    migrator.run(&executor).await
}

fn validate_schema_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let valid = match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {
            chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "invalid Postgres schema name `{name}`: must start with an ASCII letter or underscore, then only ASCII letters, digits, or underscores"
        )))
    }
}

fn quote_schema_name(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

async fn set_search_path(pool: &PgPool, path: &str) -> Result<()> {
    let sql = format!("SET search_path TO {path}");
    let mut held = Vec::new();
    while let Some(conn) = pool.try_acquire() {
        held.push(conn);
        if held.len() >= 64 {
            break;
        }
    }
    if held.is_empty() {
        sqlx::query(&sql).execute(pool).await.map_err(map_sqlx)?;
    } else {
        for mut conn in held {
            sqlx::query(&sql)
                .execute(&mut *conn)
                .await
                .map_err(map_sqlx)?;
        }
    }
    Ok(())
}

/// `pg_advisory_xact_lock` key each migration step takes inside its own transaction, so
/// migrators on other hosts apply or roll back one version at a time. The lock ends with
/// the transaction, so it cannot outlive or be lost during the work it protects.
pub const MIGRATION_LOCK_KEY: i64 = 0x6173_685f_6d69_6772;

async fn begin_locked(pool: &PgPool) -> Result<sqlx::Transaction<'static, sqlx::Postgres>> {
    let mut tx = pool.begin().await.map_err(map_sqlx)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MIGRATION_LOCK_KEY)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    Ok(tx)
}

struct PgMigrationExecutor {
    pool: PgPool,
    create_table_sql: &'static str,
}

impl PgMigrationExecutor {
    fn new(pool: PgPool, create_table_sql: &'static str) -> Self {
        Self {
            pool,
            create_table_sql,
        }
    }

    async fn init(&self) -> Result<()> {
        // Concurrent `CREATE TABLE IF NOT EXISTS` calls can still collide, so take the lock.
        let mut tx = begin_locked(&self.pool).await?;
        sqlx::query(self.create_table_sql)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)
    }
}

impl MigrationExecutor for PgMigrationExecutor {
    async fn execute_script(&self, sql: &str) -> Result<()> {
        sqlx::raw_sql(sql)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }

    async fn ensure_tracking_table(&self) -> Result<()> {
        self.init().await
    }

    async fn apply_migration(&self, sql: &str, version: &str, name: &str) -> Result<bool> {
        let mut tx = begin_locked(&self.pool).await?;
        let applied = sqlx::query("SELECT 1 FROM _ash_schema_migrations WHERE version = $1")
            .bind(version)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .is_some();
        if applied {
            return Ok(false);
        }
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        sqlx::query(
            "INSERT INTO _ash_schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
        )
        .bind(version)
        .bind(name)
        .bind(ash_core::utc_now_iso8601())
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(true)
    }

    async fn revert_migration(&self, sql: &str, version: &str) -> Result<bool> {
        let mut tx = begin_locked(&self.pool).await?;
        let latest =
            sqlx::query("SELECT 1 WHERE (SELECT MAX(version) FROM _ash_schema_migrations) = $1")
                .bind(version)
            .fetch_optional(&mut *tx)
            .await
            .map_err(map_sqlx)?
            .is_some();
        if !latest {
            return Ok(false);
        }
        sqlx::raw_sql(sql)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        sqlx::query("DELETE FROM _ash_schema_migrations WHERE version = $1")
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(map_sqlx)?;
        tx.commit().await.map_err(map_sqlx)?;
        Ok(true)
    }

    async fn applied_versions(&self) -> Result<Vec<String>> {
        let rows = sqlx::query("SELECT version FROM _ash_schema_migrations ORDER BY version ASC")
            .fetch_all(&self.pool)
            .await
            .map_err(map_sqlx)?;
        let mut versions = Vec::new();
        for r in rows {
            let v: String = r.try_get("version").map_err(map_sqlx)?;
            versions.push(v);
        }
        Ok(versions)
    }

    async fn record_migration(&self, version: &str, name: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO _ash_schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)",
        )
        .bind(version)
        .bind(name)
        .bind(ash_core::utc_now_iso8601())
        .execute(&self.pool)
        .await
        .map_err(map_sqlx)?;
        Ok(())
    }

    async fn remove_migration(&self, version: &str) -> Result<()> {
        sqlx::query("DELETE FROM _ash_schema_migrations WHERE version = $1")
            .bind(version)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
    }
}

fn bind_compiled<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments>,
    params: &'q [SqlParam],
) -> sqlx::query::Query<'q, sqlx::Postgres, sqlx::postgres::PgArguments> {
    for p in params {
        if p.is_list
            && let Value::Array(items) = &p.value
        {
            // If the array is empty or contains UUIDs, integers, strings, etc.
            if items.iter().all(|v| matches!(v, Value::Uuid(_))) {
                let uuids: Vec<Uuid> = items
                    .iter()
                    .filter_map(|v| match v {
                        Value::Uuid(u) => Some(*u),
                        _ => None,
                    })
                    .collect();
                query = query.bind(uuids);
                continue;
            } else if items.iter().all(|v| matches!(v, Value::Int(_))) {
                let ints: Vec<i64> = items
                    .iter()
                    .filter_map(|v| match v {
                        Value::Int(i) => Some(*i),
                        _ => None,
                    })
                    .collect();
                query = query.bind(ints);
                continue;
            } else if items.iter().all(|v| matches!(v, Value::String(_))) {
                let strings: Vec<String> = items
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect();
                query = query.bind(strings);
                continue;
            } else {
                // Fallback to string array
                let strings: Vec<String> = items
                    .iter()
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Uuid(u) => u.to_string(),
                        Value::Int(i) => i.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => v.to_string(),
                    })
                    .collect();
                query = query.bind(strings);
                continue;
            }
        }
        match &p.value {
            // Bind NULL with the column's type: a cached statement keeps the parameter
            // types of its first run, so a text NULL would break a later uuid value.
            Value::Null => {
                query = match p.ty {
                    Some(AttrType::Uuid) => query.bind(None::<Uuid>),
                    Some(AttrType::Integer) => query.bind(None::<i64>),
                    Some(AttrType::Boolean) => query.bind(None::<bool>),
                    Some(AttrType::Map | AttrType::Array) => query.bind(None::<serde_json::Value>),
                    _ => query.bind(None::<String>),
                };
            }
            Value::Bool(b) => {
                query = query.bind(*b);
            }
            Value::Int(i) => {
                query = query.bind(*i);
            }
            Value::Uuid(u) => {
                query = query.bind(*u);
            }
            Value::String(s) => {
                query = query.bind(s.clone());
            }
            Value::Map(_) | Value::Array(_) => {
                query = query.bind(p.value.to_plain_json());
            }
        }
    }
    query
}

fn row_to_fields(
    row: &PgRow,
    resource: &ResourceDef,
    calculations: &[String],
    aggregates: &[String],
) -> Result<FieldMap> {
    let mut map = FieldMap::new();

    for attr in resource.attributes {
        let val = extract_column_value(row, attr.name, &attr.ty);
        map.insert(attr.name.to_string(), val);
    }

    for calc_name in calculations {
        if let Some(calc) = resource.calculation(calc_name) {
            let val = extract_column_value(row, calc.name, &calc.ty);
            map.insert(calc.name.to_string(), val);
        }
    }

    for agg_name in aggregates {
        if let Some(agg) = resource.aggregate(agg_name) {
            let val = extract_column_value(row, agg.name, &agg.ty);
            map.insert(agg.name.to_string(), val);
        }
    }

    Ok(map)
}

/// Reads a column sqlx has no Rust type for, decoding Postgres's binary or text form.
fn raw_column(
    row: &PgRow,
    col_name: &str,
    binary: fn(&[u8]) -> Option<String>,
    text: fn(&str) -> Option<String>,
) -> Value {
    use sqlx::ValueRef;
    let Ok(raw) = row.try_get_raw(col_name) else {
        return Value::Null;
    };
    if raw.is_null() {
        return Value::Null;
    }
    let decoded = match raw.format() {
        sqlx::postgres::PgValueFormat::Binary => raw.as_bytes().ok().and_then(binary),
        sqlx::postgres::PgValueFormat::Text => raw.as_str().ok().and_then(text),
    };
    decoded.map(Value::String).unwrap_or(Value::Null)
}

/// Binary `inet`: family, prefix bits, cidr flag, address length, then the address bytes.
fn decode_inet(bytes: &[u8]) -> Option<String> {
    let [family, bits, _, len, addr @ ..] = bytes else {
        return None;
    };
    let addr = match (family, *len as usize) {
        (2, 4) => std::net::IpAddr::from(<[u8; 4]>::try_from(addr).ok()?),
        (3, 16) => std::net::IpAddr::from(<[u8; 16]>::try_from(addr).ok()?),
        _ => return None,
    };
    Some(ash_core::format_inet(addr, *bits))
}

/// Binary pgvector: dimensions (u16), an unused u16, then big-endian f32 values.
fn decode_vector(bytes: &[u8]) -> Option<String> {
    let dimensions = u16::from_be_bytes([*bytes.first()?, *bytes.get(1)?]) as usize;
    let data = bytes.get(4..)?;
    if data.len() != dimensions * 4 {
        return None;
    }
    let values: Vec<f32> = data
        .chunks_exact(4)
        .map(|chunk| f32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    Some(ash_core::format_vector(&values))
}

fn extract_column_value(row: &PgRow, col_name: &str, ty: &ash_core::AttrType) -> Value {
    match ty {
        ash_core::AttrType::Inet => raw_column(row, col_name, decode_inet, |text| {
            ash_core::Inet::parse(text).ok().map(|inet| inet.as_str().to_string())
        }),
        ash_core::AttrType::Vector { .. } => raw_column(row, col_name, decode_vector, |text| {
            ash_core::parse_vector(text)
                .ok()
                .map(|values| ash_core::format_vector(&values))
        }),
        ash_core::AttrType::Uuid => {
            if let Ok(Some(u)) = row.try_get::<Option<Uuid>, _>(col_name) {
                Value::Uuid(u)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Uuid::parse_str(&s).map(Value::Uuid).unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::String
        | ash_core::AttrType::CiString
        | ash_core::AttrType::Atom { .. } => {
            if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::UtcDatetime { precision } => {
            if let Ok(Some(dt)) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(col_name)
            {
                Value::String(precision.format(dt))
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(precision.normalize(&s).unwrap_or(s))
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Decimal => {
            if let Ok(Some(n)) = row.try_get::<Option<rust_decimal::Decimal>, _>(col_name) {
                Value::String(n.to_string())
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Binary => {
            if let Ok(Some(bytes)) = row.try_get::<Option<Vec<u8>>, _>(col_name) {
                Value::String(ash_core::Binary::from_bytes(bytes).encode())
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Date => {
            if let Ok(Some(date)) = row.try_get::<Option<chrono::NaiveDate>, _>(col_name) {
                Value::String(date.format("%Y-%m-%d").to_string())
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Float => {
            if let Ok(Some(n)) = row.try_get::<Option<f64>, _>(col_name) {
                Value::String(n.to_string())
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Integer => {
            if let Ok(Some(i)) = row.try_get::<Option<i64>, _>(col_name) {
                Value::Int(i)
            } else if let Ok(Some(i)) = row.try_get::<Option<i32>, _>(col_name) {
                Value::Int(i as i64)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Boolean => {
            if let Ok(Some(b)) = row.try_get::<Option<bool>, _>(col_name) {
                Value::Bool(b)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Map => {
            if let Ok(Some(json_val)) = row.try_get::<Option<serde_json::Value>, _>(col_name) {
                Value::from_plain_json(json_val)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                serde_json::from_str::<serde_json::Value>(&s)
                    .map(Value::from_plain_json)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Array => {
            if let Ok(Some(json_val)) = row.try_get::<Option<serde_json::Value>, _>(col_name) {
                Value::from_plain_json(json_val)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                serde_json::from_str::<serde_json::Value>(&s)
                    .map(Value::from_plain_json)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
    }
}

fn map_sqlx(err: sqlx::Error) -> Error {
    Error::DataLayer(err.to_string())
}

fn map_sqlx_resource(err: sqlx::Error, resource: &ResourceDef) -> Error {
    if let sqlx::Error::Database(ref db_err) = err
        && let Some(code) = db_err.code()
    {
        match code.as_ref() {
            "23505" => {
                let constraint = db_err.constraint().unwrap_or_default();
                for id in resource.identities {
                    let expected_idx = format!("idx_{}_{}", resource.table_name(), id.name);
                    if constraint == expected_idx || constraint.contains(id.name) {
                        return Error::IdentityConflict {
                            identity: id.name,
                            fields: id.keys.iter().map(|s| s.to_string()).collect(),
                            message: id
                                .message
                                .unwrap_or("Unique constraint violation")
                                .to_string(),
                        };
                    }
                }
                let id_name = resource
                    .identities
                    .first()
                    .map(|i| i.name)
                    .unwrap_or("unique_constraint");
                return Error::IdentityConflict {
                    identity: id_name,
                    fields: Vec::new(),
                    message: "unique constraint violation".to_string(),
                };
            }
            "23503" => {
                return Error::Invalid(
                    "foreign key violation: referenced record does not exist".to_string(),
                );
            }
            "23514" => {
                return Error::Validation {
                    field: "validation".to_string(),
                    message: db_err.message().to_string(),
                };
            }
            "40P01" => {
                return Error::DataLayer("deadlock detected".to_string());
            }
            _ => {}
        }
    }
    Error::DataLayer(err.to_string())
}
