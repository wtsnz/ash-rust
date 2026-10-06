//! # `ash-postgres`
//!
//! PostgreSQL data layer implementation for `ash-rust`, on `tokio-postgres` with a
//! `deadpool-postgres` pool. Provides single-roundtrip writes via `RETURNING *`, native
//! error code translation, schema multitenancy, and transaction savepoint management.

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
use deadpool_postgres::{Manager, ManagerConfig, Object, Pool, RecyclingMethod};
use tokio::sync::Mutex;
use tokio_postgres::types::{FromSql, ToSql, Type};
use tokio_postgres::{NoTls, Row};
use uuid::Uuid;

/// Connections a pool opens at most.
const POOL_SIZE: usize = 20;

/// How a pool is sized, and how long a statement waits for a connection.
///
/// The default is 20 connections and no wait limit, so a statement waits for one as long as it
/// takes and a queue of them can grow without bound under load. Ecto's pool, in Ash, sheds
/// instead: a request that has waited past its `queue_target` (50 ms, doubled once the pool has
/// been slow for an interval) is dropped before it reaches the database. Setting a
/// `wait_timeout` fails a statement that has waited that long with a pool error, which a
/// server can answer as overloaded, so a deep queue can't build.
#[derive(Clone, Copy, Debug)]
pub struct PoolSettings {
    /// Connections the pool opens at most.
    pub size: usize,
    /// How long a statement waits for a free connection before it fails; `None` waits as
    /// long as it takes.
    pub wait_timeout: Option<std::time::Duration>,
}

impl Default for PoolSettings {
    fn default() -> Self {
        Self {
            size: POOL_SIZE,
            wait_timeout: None,
        }
    }
}

#[derive(Clone)]
enum PostgresSource {
    /// The pool, and the settings it connects with, for pools of its own (a tenant's
    /// migrations) to start from.
    Pool(Pool, Option<Arc<tokio_postgres::Config>>),
    Tx(Arc<Mutex<Object>>),
}

/// PostgreSQL data layer for Ash.
#[derive(Clone)]
pub struct Postgres {
    source: PostgresSource,
}

impl std::fmt::Debug for Postgres {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let source = match &self.source {
            PostgresSource::Pool(pool, _) => format!("{:?}", pool.status()),
            PostgresSource::Tx(_) => "transaction".to_string(),
        };
        f.debug_struct("Postgres").field("source", &source).finish()
    }
}

/// Why a statement failed: no connection to run it on, or the database refused it.
enum Failure {
    Pool(String),
    Db(tokio_postgres::Error),
}

/// A connection one statement runs on: one checked out of the pool for it, or the
/// transaction's. It lives on the stack for one statement, so it isn't boxed, which would
/// allocate for every statement.
#[allow(clippy::large_enum_variant)]
enum Conn<'a> {
    Pooled(Object),
    Tx(tokio::sync::MutexGuard<'a, Object>),
}

impl std::ops::Deref for Conn<'_> {
    type Target = Object;

    fn deref(&self) -> &Object {
        match self {
            Conn::Pooled(object) => object,
            Conn::Tx(guard) => guard,
        }
    }
}

/// A statement's parameters as Postgres receives them, each with the type it's declared
/// with, as sqlx declares a bound value's: the SQL's casts (`$1::timestamptz`) take it
/// from there, and a statement cached for one value's type serves the next.
struct Params {
    values: Vec<Box<dyn ToSql + Sync + Send>>,
    types: Vec<Type>,
}

impl Params {
    fn of(params: &[SqlParam]) -> Self {
        let mut values: Vec<Box<dyn ToSql + Sync + Send>> = Vec::with_capacity(params.len());
        let mut types = Vec::with_capacity(params.len());
        for p in params {
            let (value, ty): (Box<dyn ToSql + Sync + Send>, Type) = match &p.value {
                Value::Array(items) if p.is_list => {
                    if items.iter().all(|v| matches!(v, Value::Uuid(_))) {
                        let uuids: Vec<Uuid> = items.iter().filter_map(Value::as_uuid).collect();
                        (Box::new(uuids), Type::UUID_ARRAY)
                    } else if items.iter().all(|v| matches!(v, Value::Int(_))) {
                        let ints: Vec<i64> = items
                            .iter()
                            .filter_map(|v| match v {
                                Value::Int(i) => Some(*i),
                                _ => None,
                            })
                            .collect();
                        (Box::new(ints), Type::INT8_ARRAY)
                    } else {
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
                        (Box::new(strings), Type::TEXT_ARRAY)
                    }
                }
                // NULL is declared with the column's type: a cached statement keeps the
                // parameter types of its first run, so a text NULL would break a later
                // uuid value.
                Value::Null => match p.ty {
                    Some(AttrType::Uuid) => (Box::new(None::<Uuid>), Type::UUID),
                    Some(AttrType::Integer) => (Box::new(None::<i64>), Type::INT8),
                    Some(AttrType::Boolean) => (Box::new(None::<bool>), Type::BOOL),
                    Some(AttrType::Map | AttrType::Array { .. } | AttrType::Embedded(_) | AttrType::TypedMap { .. } | AttrType::Union { .. }) => {
                        (Box::new(None::<serde_json::Value>), Type::JSONB)
                    }
                    _ => (Box::new(None::<String>), Type::TEXT),
                },
                Value::Bool(b) => (Box::new(*b), Type::BOOL),
                Value::Int(i) => (Box::new(*i), Type::INT8),
                Value::Float(n) => (Box::new(*n), Type::FLOAT8),
                Value::Uuid(u) => (Box::new(*u), Type::UUID),
                Value::String(s) => (Box::new(s.clone()), Type::TEXT),
                Value::Map(_) | Value::Array(_) => (Box::new(p.value.to_plain_json()), Type::JSONB),
            };
            values.push(value);
            types.push(ty);
        }
        Self { values, types }
    }

    fn refs(&self) -> Vec<&(dyn ToSql + Sync)> {
        self.values.iter().map(|value| value.as_ref() as &(dyn ToSql + Sync)).collect()
    }
}

impl Postgres {
    /// Creates a [`Postgres`] instance from an existing pool. Migrating several schemas
    /// ([`migrate_schemas`](Self::migrate_schemas)) needs the settings it connects with:
    /// use [`connect`](Self::connect) or [`connect_with`](Self::connect_with).
    pub fn new(pool: Pool) -> Self {
        Self {
            source: PostgresSource::Pool(pool, None),
        }
    }

    /// Connects to PostgreSQL using a connection string.
    pub async fn connect(url: &str) -> Result<Self> {
        let config = tokio_postgres::Config::from_str(url).map_err(map_pg)?;
        Self::connect_with(config).await
    }

    /// Connects to PostgreSQL with custom settings.
    ///
    /// A connection isn't checked with a round trip as it's checked out or returned, as
    /// Postgrex doesn't check one for Ash: each statement checks one out, so a check was a
    /// round trip per statement. A connection the server closed is noticed locally and
    /// replaced; one that dies unnoticed while idle fails the next statement.
    pub async fn connect_with(config: tokio_postgres::Config) -> Result<Self> {
        Self::connect_with_pool(config, PoolSettings::default()).await
    }

    /// [`connect_with`](Self::connect_with), with a pool sized and bounded as `settings` say.
    pub async fn connect_with_pool(config: tokio_postgres::Config, settings: PoolSettings) -> Result<Self> {
        let pool = pool_with(&config, settings)?;
        // Connect once now, so an unreachable database fails here, as sqlx's did.
        drop(pool.get().await.map_err(|e| Error::DataLayer(e.to_string()))?);
        Ok(Self {
            source: PostgresSource::Pool(pool, Some(Arc::new(config))),
        })
    }

    /// Returns a reference to the underlying pool, if not inside an active transaction.
    pub fn pool(&self) -> Option<&Pool> {
        match &self.source {
            PostgresSource::Pool(pool, _) => Some(pool),
            PostgresSource::Tx(_) => None,
        }
    }

    /// Runs `sql`, one statement or several, with no parameters: setup and maintenance
    /// SQL such as `TRUNCATE` or `CREATE SCHEMA`. In a transaction, it runs there.
    pub async fn execute_sql(&self, sql: &str) -> Result<()> {
        let conn = self.conn().await.map_err(map_failure)?;
        conn.batch_execute(sql).await.map_err(map_pg)
    }

    async fn conn(&self) -> std::result::Result<Conn<'_>, Failure> {
        match &self.source {
            PostgresSource::Pool(pool, _) => pool
                .get()
                .await
                .map(Conn::Pooled)
                .map_err(|e| Failure::Pool(pool_error(&e))),
            PostgresSource::Tx(conn) => Ok(Conn::Tx(conn.lock().await)),
        }
    }

    /// The rows `compiled` returns, from its statement prepared once per connection.
    async fn rows(&self, compiled: &CompiledSql) -> std::result::Result<Vec<Row>, Failure> {
        let params = Params::of(&compiled.params);
        let conn = self.conn().await?;
        let statement = conn
            .prepare_typed_cached(&compiled.sql, &params.types)
            .await
            .map_err(Failure::Db)?;
        conn.query(&statement, &params.refs()).await.map_err(Failure::Db)
    }

    /// The number of rows `compiled` changed.
    async fn affected(&self, compiled: &CompiledSql) -> std::result::Result<u64, Failure> {
        let params = Params::of(&compiled.params);
        let conn = self.conn().await?;
        let statement = conn
            .prepare_typed_cached(&compiled.sql, &params.types)
            .await
            .map_err(Failure::Db)?;
        conn.execute(&statement, &params.refs()).await.map_err(Failure::Db)
    }

    async fn execute_compiled(&self, compiled: &CompiledSql) -> Result<u64> {
        self.affected(compiled).await.map_err(map_failure)
    }

    async fn execute_compiled_resource(&self, compiled: &CompiledSql, resource: &ResourceDef) -> Result<u64> {
        self.affected(compiled).await.map_err(|e| map_failure_resource(e, resource))
    }

    async fn fetch_one_resource(&self, compiled: &CompiledSql, resource: &ResourceDef) -> Result<Row> {
        self.fetch_optional_resource(compiled, resource)
            .await?
            .ok_or_else(|| Error::DataLayer("no rows returned by a query that expected to return at least one row".into()))
    }

    async fn fetch_all_resource(&self, compiled: &CompiledSql, resource: &ResourceDef) -> Result<Vec<Row>> {
        self.rows(compiled).await.map_err(|e| map_failure_resource(e, resource))
    }

    async fn fetch_optional_resource(&self, compiled: &CompiledSql, resource: &ResourceDef) -> Result<Option<Row>> {
        Ok(self.fetch_all_resource(compiled, resource).await?.into_iter().next())
    }

    async fn fetch_all(&self, compiled: &CompiledSql) -> Result<Vec<Row>> {
        self.rows(compiled).await.map_err(map_failure)
    }

    /// [`fetch_all`](Self::fetch_all), keeping the database's error for the caller to read.
    async fn fetch_all_raw(&self, compiled: &CompiledSql) -> std::result::Result<Vec<Row>, Failure> {
        self.rows(compiled).await
    }

    /// Runs one statement with no parameters, returning the rows it changed.
    async fn execute_raw(&self, sql: &str) -> Result<u64> {
        let conn = self.conn().await.map_err(map_failure)?;
        conn.execute(sql, &[]).await.map_err(map_pg)
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
                    + self
                        .execute_raw(&ash_sql::install::record_changed_statement(table, name, up))
                        .await?;
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

    /// Create each Postgres schema if needed and migrate the same history into it, each
    /// through a connection whose `search_path` starts with that schema. Schema names
    /// must match `[A-Za-z_][A-Za-z0-9_]*`.
    pub async fn migrate_schemas(
        &self,
        schemas: &[&str],
        migrations_dir: impl AsRef<Path>,
    ) -> Result<()> {
        let PostgresSource::Pool(pool, config) = &self.source else {
            return Err(Error::DataLayer("Migration requires a connection pool".into()));
        };
        let config = config.as_ref().ok_or_else(|| {
            Error::DataLayer("Migrating schemas needs the pool's connection settings: connect with Postgres::connect".into())
        })?;
        let dir = migrations_dir.as_ref();
        for name in schemas {
            validate_schema_name(name)?;
            let quoted = quote_schema_name(name);
            let conn = pool.get().await.map_err(|e| Error::DataLayer(e.to_string()))?;
            conn.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {quoted}"))
                .await
                .map_err(map_pg)?;
            drop(conn);
            // `public` keeps database-wide extension types such as `citext` visible.
            let mut tenant_config = (**config).clone();
            let options = match config.get_options() {
                Some(options) => format!("{options} -c search_path={name},public"),
                None => format!("-c search_path={name},public"),
            };
            tenant_config.options(&options);
            let tenant_pool = pool_of(&tenant_config, 1)?;
            let migrated = migrate(&tenant_pool, dir).await;
            tenant_pool.close();
            migrated?;
        }
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
    async fn in_transaction<T, F, Fut>(&self, work: F) -> Result<T>
    where
        Self: Sized,
        F: FnOnce(Option<Self>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        TransactionSupport::transaction(self, move |tx| work(Some(tx.clone()))).await
    }

    async fn create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        _id: Value,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_insert(resource, &fields)?;

        // Postgres supports RETURNING * for single-roundtrip writes!
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource)
    }

    async fn update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Value,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_update(resource, id.clone(), &fields)?;

        // Postgres RETURNING * executes update and returns the new row
        let opt_row = self.fetch_optional_resource(&compiled, resource).await?;
        match opt_row {
            Some(row) => row_to_fields(&row, resource),
            None => Err(Error::NotFound),
        }
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: Value) -> Result<()> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_delete(resource, id)?;
        let affected = self.execute_compiled(&compiled).await?;
        if affected == 0 {
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
        let plan = RowPlan::new(&rows, resource, query);
        rows.iter().map(|row| plan.read(row)).collect()
    }

    fn can_run_query_per_key(&self, _resource: &ResourceDef, _by: &ash_core::PerKey<'_>) -> bool {
        true
    }

    /// The read once per key, in one statement with a lateral join (see
    /// [`QueryCompiler::compile_select_per_key`]), as AshPostgres loads a relationship.
    async fn run_query_per_key(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        by: &ash_core::PerKey<'_>,
        keys: &[Value],
    ) -> Result<Vec<Vec<FieldMap>>> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_select_per_key(resource, query, by, keys.to_vec())?;
        let rows = self.fetch_all(&compiled).await?;
        // Each row's key, by its place in `keys`, so a key given twice gets its rows twice.
        let mut per_key = vec![Vec::new(); keys.len()];
        let plan = RowPlan::new(&rows, resource, query);
        for row in &rows {
            let ord: i64 = row.try_get("__ash_ord").map_err(map_pg)?;
            if let Some(rows) = usize::try_from(ord - 1).ok().and_then(|i| per_key.get_mut(i)) {
                rows.push(plan.read(row)?);
            }
        }
        Ok(per_key)
    }

    async fn count(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<usize> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let rows = self.fetch_all(&compiler.compile_count(resource, query)?).await?;
        let count: i64 = rows
            .first()
            .ok_or_else(|| Error::DataLayer("COUNT returned no row".into()))?
            .try_get(0)
            .map_err(map_pg)?;
        Ok(count as usize)
    }

    /// The page and its count down one connection together: tokio-postgres pipelines
    /// statements sent at once on a connection, so both go out before either answers.
    async fn run_query_with_count(
        &self,
        resource: &ResourceDef,
        page: &CompiledQuery,
        count: &CompiledQuery,
    ) -> Result<(Vec<FieldMap>, usize)> {
        let dialect = PostgresDialect;
        let page_sql = QueryCompiler::new(&dialect).compile_select(resource, page)?;
        let count_sql = QueryCompiler::new(&dialect).compile_count(resource, count)?;
        let (page_params, count_params) = (Params::of(&page_sql.params), Params::of(&count_sql.params));
        let conn = self.conn().await.map_err(map_failure)?;
        let (page_statement, count_statement) = futures_util::future::try_join(
            conn.prepare_typed_cached(&page_sql.sql, &page_params.types),
            conn.prepare_typed_cached(&count_sql.sql, &count_params.types),
        )
        .await
        .map_err(map_pg)?;
        let (rows, counted) = futures_util::future::try_join(
            conn.query(&page_statement, &page_params.refs()),
            conn.query_one(&count_statement, &count_params.refs()),
        )
        .await
        .map_err(map_pg)?;
        let plan = RowPlan::new(&rows, resource, page);
        let records = rows.iter().map(|row| plan.read(row)).collect::<Result<Vec<_>>>()?;
        let counted: i64 = counted.try_get(0).map_err(map_pg)?;
        Ok((records, counted as usize))
    }

    fn can_update_atomically(&self, _resource: &ResourceDef) -> bool {
        true
    }

    /// The update as one statement (see [`QueryCompiler::compile_atomic_update`]). A
    /// condition that holds raises through `ash_raise_error`, which this turns back into
    /// the condition's error, from the record's values it reports.
    async fn update_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        update: &ash_core::AtomicUpdate,
    ) -> Result<Vec<FieldMap>> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_atomic_update(resource, query, update)?;
        let rows = match self.fetch_all_raw(&compiled).await {
            Ok(rows) => rows,
            Err(err) => {
                return Err(raised_error(&err, resource, &update.conditions).unwrap_or_else(|| map_failure_resource(err, resource)));
            }
        };
        rows_to_fields(&rows, resource)
    }

    fn can_destroy_atomically(&self, _resource: &ResourceDef) -> bool {
        true
    }

    /// The destroy as one statement (see [`QueryCompiler::compile_atomic_destroy`]), its
    /// conditions raised as an atomic update's are.
    async fn destroy_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        conditions: &[ash_core::AtomicCondition],
    ) -> Result<Vec<FieldMap>> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_atomic_destroy(resource, query, conditions)?;
        let rows = match self.fetch_all_raw(&compiled).await {
            Ok(rows) => rows,
            Err(err) => return Err(raised_error(&err, resource, conditions).unwrap_or_else(|| map_failure_resource(err, resource))),
        };
        rows_to_fields(&rows, resource)
    }

    async fn upsert(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        _id: Value,
        fields: FieldMap,
        identity: &ash_core::IdentityDef,
        update_fields: &[String],
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_upsert(resource, &fields, identity, update_fields)?;

        // Single-roundtrip write with RETURNING *
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource)
    }

    async fn bulk_create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Value, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect).with_tenant(tenant);
        let compiled = compiler.compile_bulk_insert(resource, &rows)?;
        let pg_rows = self.fetch_all_resource(&compiled, resource).await?;
        rows_to_fields(&pg_rows, resource)
    }

    /// Writes the batch in one `UPDATE … FROM (VALUES …)`, each row writing only the
    /// columns it changes, so a batch is one round trip however its rows differ. Rows that
    /// change nothing, and resources with optimistic locking, which checks each row's
    /// version, go one at a time.
    async fn bulk_update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Value, FieldMap)>,
    ) -> Result<Vec<Result<FieldMap>>> {
        let mut results: Vec<Option<Result<FieldMap>>> = (0..rows.len()).map(|_| None).collect();
        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?
            .name;
        let writes = |fields: &FieldMap| {
            resource
                .attributes
                .iter()
                .any(|attr| !attr.primary_key && fields.contains_key(attr.name))
        };
        let mut together = Vec::new();
        for (i, (id, fields)) in rows.iter().enumerate() {
            if !writes(fields) {
                results[i] = Some(self.update(resource, tenant, id.clone(), fields.clone()).await);
            } else {
                together.push(i);
            }
        }
        // Every column some row in the batch changes, in the resource's order.
        let columns: Vec<&str> = resource
            .attributes
            .iter()
            .filter(|attr| !attr.primary_key && together.iter().any(|&i| rows[i].1.contains_key(attr.name)))
            .map(|attr| attr.name)
            .collect();
        let dialect = PostgresDialect;
        // Postgres binds at most 65,535 parameters to a statement.
        let per_statement = (65_535 / (columns.len() + 1)).max(1);
        for batch in together.chunks(per_statement) {
            let batch_rows: Vec<(Value, &FieldMap)> =
                batch.iter().map(|&i| (rows[i].0.clone(), &rows[i].1)).collect();
            let compiled = QueryCompiler::new(&dialect)
                .with_tenant(tenant)
                .compile_bulk_update(resource, &columns, &batch_rows)?;
            let mut stored: std::collections::HashMap<Value, FieldMap> =
                rows_to_fields(&self.fetch_all_resource(&compiled, resource).await?, resource)?
                    .into_iter()
                    .map(|fields| {
                        let id = match fields.get(pk) {
                            Some(id) if !id.is_null() => id.clone(),
                            _ => return Err(Error::DataLayer(format!("{} row without its key", resource.name))),
                        };
                        Ok((id, fields))
                    })
                    .collect::<Result<_>>()?;
            for &i in batch {
                results[i] = Some(stored.remove(&rows[i].0).ok_or(Error::NotFound));
            }
        }
        Ok(results
            .into_iter()
            .map(|result| result.expect("every row has a result"))
            .collect())
    }

    async fn bulk_destroy(&self, resource: &ResourceDef, tenant: Option<&str>, ids: &[Value]) -> Result<()> {
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
            PostgresSource::Pool(pool, _) => {
                let conn = pool.get().await.map_err(|e| Error::DataLayer(e.to_string()))?;
                conn.batch_execute("BEGIN").await.map_err(map_pg)?;
                let conn = Arc::new(Mutex::new(conn));
                let tx_pg = Postgres {
                    source: PostgresSource::Tx(Arc::clone(&conn)),
                };
                match f(&tx_pg).await {
                    Ok(val) => {
                        conn.lock().await.batch_execute("COMMIT").await.map_err(map_pg)?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = conn.lock().await.batch_execute("ROLLBACK").await;
                        Err(err)
                    }
                }
            }
            PostgresSource::Tx(conn) => {
                let sp_name = format!("sp_{}", Uuid::new_v4().simple());
                conn.lock()
                    .await
                    .batch_execute(&format!("SAVEPOINT {sp_name}"))
                    .await
                    .map_err(map_pg)?;
                match f(self).await {
                    Ok(val) => {
                        conn.lock()
                            .await
                            .batch_execute(&format!("RELEASE SAVEPOINT {sp_name}"))
                            .await
                            .map_err(map_pg)?;
                        Ok(val)
                    }
                    Err(err) => {
                        let _ = conn
                            .lock()
                            .await
                            .batch_execute(&format!("ROLLBACK TO SAVEPOINT {sp_name}"))
                            .await;
                        Err(err)
                    }
                }
            }
        }
    }
}

/// A pool of up to `size` connections made with `config`. A connection is reused without
/// a round trip to check it (`RecyclingMethod::Fast`), as Postgrex reuses one.
fn pool_of(config: &tokio_postgres::Config, size: usize) -> Result<Pool> {
    pool_with(config, PoolSettings { size, ..PoolSettings::default() })
}

fn pool_with(config: &tokio_postgres::Config, settings: PoolSettings) -> Result<Pool> {
    let manager = Manager::from_config(
        config.clone(),
        NoTls,
        ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        },
    );
    let mut builder = Pool::builder(manager).max_size(settings.size);
    if settings.wait_timeout.is_some() {
        // A timeout needs the runtime that will time it.
        builder = builder.runtime(deadpool_postgres::Runtime::Tokio1).wait_timeout(settings.wait_timeout);
    }
    builder.build().map_err(|e| Error::DataLayer(e.to_string()))
}

/// What a failed checkout says: that the pool had no connection free in time, or why not.
fn pool_error(error: &deadpool_postgres::PoolError) -> String {
    match error {
        deadpool_postgres::PoolError::Timeout(deadpool_postgres::TimeoutType::Wait) => {
            "connection pool exhausted: no connection became free within the wait limit".to_string()
        }
        other => other.to_string(),
    }
}

/// Runs declarative migrations against a PostgreSQL connection pool.
pub async fn migrate(pool: &Pool, migrations_dir: impl AsRef<Path>) -> Result<Vec<String>> {
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

/// `pg_advisory_xact_lock` key each migration step takes inside its own transaction, so
/// migrators on other hosts apply or roll back one version at a time. The lock ends with
/// the transaction, so it cannot outlive or be lost during the work it protects.
pub const MIGRATION_LOCK_KEY: i64 = 0x6173_685f_6d69_6772;

/// Text, as each value in a migration's bookkeeping is.
const TEXT: Type = Type::TEXT;

struct PgMigrationExecutor {
    pool: Pool,
    create_table_sql: &'static str,
}

impl PgMigrationExecutor {
    fn new(pool: Pool, create_table_sql: &'static str) -> Self {
        Self {
            pool,
            create_table_sql,
        }
    }

    async fn conn(&self) -> Result<Object> {
        self.pool.get().await.map_err(|e| Error::DataLayer(e.to_string()))
    }

    async fn init(&self) -> Result<()> {
        // Concurrent `CREATE TABLE IF NOT EXISTS` calls can still collide, so take the lock.
        let mut conn = self.conn().await?;
        let tx = locked(&mut conn).await?;
        tx.batch_execute(self.create_table_sql).await.map_err(map_pg)?;
        tx.commit().await.map_err(map_pg)
    }
}

/// A transaction holding the migration lock.
async fn locked(conn: &mut Object) -> Result<deadpool_postgres::Transaction<'_>> {
    let tx = conn.transaction().await.map_err(map_pg)?;
    tx.execute("SELECT pg_advisory_xact_lock($1)", &[&MIGRATION_LOCK_KEY])
        .await
        .map_err(map_pg)?;
    Ok(tx)
}

/// Whether `sql`, with `version` for its one parameter, returns a row.
async fn any_row(tx: &deadpool_postgres::Transaction<'_>, sql: &str, version: &str) -> Result<bool> {
    let statement = tx.prepare_typed(sql, &[TEXT]).await.map_err(map_pg)?;
    Ok(!tx.query(&statement, &[&version]).await.map_err(map_pg)?.is_empty())
}

const RECORD_MIGRATION: &str =
    "INSERT INTO _ash_schema_migrations (version, name, applied_at) VALUES ($1, $2, $3)";
const FORGET_MIGRATION: &str = "DELETE FROM _ash_schema_migrations WHERE version = $1";

impl MigrationExecutor for PgMigrationExecutor {
    async fn execute_script(&self, sql: &str) -> Result<()> {
        self.conn().await?.batch_execute(sql).await.map_err(map_pg)
    }

    async fn ensure_tracking_table(&self) -> Result<()> {
        self.init().await
    }

    async fn apply_migration(&self, sql: &str, version: &str, name: &str) -> Result<bool> {
        let mut conn = self.conn().await?;
        let tx = locked(&mut conn).await?;
        if any_row(&tx, "SELECT 1 FROM _ash_schema_migrations WHERE version = $1", version).await? {
            return Ok(false);
        }
        tx.batch_execute(sql).await.map_err(map_pg)?;
        let record = tx.prepare_typed(RECORD_MIGRATION, &[TEXT, TEXT, TEXT]).await.map_err(map_pg)?;
        tx.execute(&record, &[&version, &name, &ash_core::utc_now_iso8601()])
            .await
            .map_err(map_pg)?;
        tx.commit().await.map_err(map_pg)?;
        Ok(true)
    }

    async fn revert_migration(&self, sql: &str, version: &str) -> Result<bool> {
        let mut conn = self.conn().await?;
        let tx = locked(&mut conn).await?;
        let latest = "SELECT 1 WHERE (SELECT MAX(version) FROM _ash_schema_migrations) = $1";
        if !any_row(&tx, latest, version).await? {
            return Ok(false);
        }
        tx.batch_execute(sql).await.map_err(map_pg)?;
        let forget = tx.prepare_typed(FORGET_MIGRATION, &[TEXT]).await.map_err(map_pg)?;
        tx.execute(&forget, &[&version]).await.map_err(map_pg)?;
        tx.commit().await.map_err(map_pg)?;
        Ok(true)
    }

    async fn applied_versions(&self) -> Result<Vec<String>> {
        let rows = self
            .conn()
            .await?
            .query("SELECT version FROM _ash_schema_migrations ORDER BY version ASC", &[])
            .await
            .map_err(map_pg)?;
        rows.iter().map(|row| row.try_get("version").map_err(map_pg)).collect()
    }

    async fn record_migration(&self, version: &str, name: &str) -> Result<()> {
        let conn = self.conn().await?;
        let record = conn.prepare_typed(RECORD_MIGRATION, &[TEXT, TEXT, TEXT]).await.map_err(map_pg)?;
        conn.execute(&record, &[&version, &name, &ash_core::utc_now_iso8601()])
            .await
            .map_err(map_pg)?;
        Ok(())
    }

    async fn remove_migration(&self, version: &str) -> Result<()> {
        let conn = self.conn().await?;
        let forget = conn.prepare_typed(FORGET_MIGRATION, &[TEXT]).await.map_err(map_pg)?;
        conn.execute(&forget, &[&version]).await.map_err(map_pg)?;
        Ok(())
    }
}

/// A record written with `RETURNING *`.
fn row_to_fields(row: &Row, resource: &ResourceDef) -> Result<FieldMap> {
    let query = CompiledQuery::default();
    RowPlan::new(std::slice::from_ref(row), resource, &query).read(row)
}

/// Records written with `RETURNING *`, the plan worked out once for them all.
fn rows_to_fields(rows: &[Row], resource: &ResourceDef) -> Result<Vec<FieldMap>> {
    let query = CompiledQuery::default();
    let plan = RowPlan::new(rows, resource, &query);
    rows.iter().map(|row| plan.read(row)).collect()
}

/// Where each field a result's rows hold sits, and its type, worked out once for the
/// result: tokio-postgres finds a column by name by searching every column, so looking
/// each field up by name in each row cost a search per field per row.
struct RowPlan<'q> {
    resource: &'q ResourceDef,
    query: &'q CompiledQuery,
    /// Each field read: its name, type, and column, if the result has it.
    fields: Vec<(&'static str, AttrType, Option<usize>)>,
}

impl<'q> RowPlan<'q> {
    /// The plan for `rows` that `query` read: the attributes it selected, and the
    /// calculations and aggregates it asked for.
    fn new(rows: &[Row], resource: &'q ResourceDef, query: &'q CompiledQuery) -> Self {
        let columns = rows.first().map(Row::columns).unwrap_or_default();
        let column = |name: &str| columns.iter().position(|col| col.name() == name);
        let mut fields = Vec::with_capacity(resource.attributes.len() + query.calculations.len() + query.aggregates.len());
        for attr in resource.attributes.iter().filter(|attr| query.reads(resource, attr)) {
            fields.push((attr.name, attr.ty, column(attr.name)));
        }
        for calc_name in &query.calculations {
            if let Some(calc) = resource.calculation(calc_name).filter(|calc| !calc.expr.is_custom()) {
                fields.push((calc.name, calc.ty, column(calc.name)));
            }
        }
        for agg_name in &query.aggregates {
            if let Some(agg) = resource.aggregate(agg_name) {
                fields.push((agg.name, agg.ty, column(agg.name)));
            }
        }
        Self { resource, query, fields }
    }

    /// A row as the record it holds, with what only Rust computes from it.
    fn read(&self, row: &Row) -> Result<FieldMap> {
        let mut map = FieldMap::with_capacity(self.fields.len());
        for (name, ty, column) in &self.fields {
            let value = column.map_or(Value::Null, |idx| extract_column_value(row, idx, ty));
            map.insert(name.to_string(), value);
        }
        for calc in self.query.custom_calculations(self.resource) {
            let args = self.query.calculation_args.get(calc.name).cloned().unwrap_or_default();
            ash_core::apply_named_with_args(self.resource, &mut map, calc.name, &args)?;
        }
        Ok(map)
    }
}

/// A column's bytes as Postgres sent them (in binary), whatever its type: for the types
/// with no Rust type to decode into.
struct Raw(Vec<u8>);

impl<'a> FromSql<'a> for Raw {
    fn from_sql(_: &Type, raw: &'a [u8]) -> std::result::Result<Self, Box<dyn std::error::Error + Sync + Send>> {
        Ok(Raw(raw.to_vec()))
    }

    fn accepts(_: &Type) -> bool {
        true
    }
}

/// A column as `T`, if it's not NULL and of a type `T` decodes.
fn get<'a, T: FromSql<'a>>(row: &'a Row, idx: usize) -> Option<T> {
    row.try_get::<_, Option<T>>(idx).ok().flatten()
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

fn extract_column_value(row: &Row, column: usize, ty: &ash_core::AttrType) -> Value {
    let text = |row: &Row| get::<String>(row, column);
    match ty {
        ash_core::AttrType::Inet => get::<Raw>(row, column)
            .and_then(|raw| decode_inet(&raw.0))
            .map(Value::String)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Vector { .. } => get::<Raw>(row, column)
            .and_then(|raw| decode_vector(&raw.0))
            .map(Value::String)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Uuid => get::<Uuid>(row, column)
            .map(Value::Uuid)
            .or_else(|| text(row).and_then(|s| Uuid::parse_str(&s).ok()).map(Value::Uuid))
            .unwrap_or(Value::Null),
        ash_core::AttrType::String | ash_core::AttrType::CiString | ash_core::AttrType::Atom { .. } => {
            text(row).map(Value::String).unwrap_or(Value::Null)
        }
        ash_core::AttrType::UtcDatetime { precision } => get::<chrono::DateTime<chrono::Utc>>(row, column)
            .or_else(|| get::<chrono::NaiveDateTime>(row, column).map(|dt| dt.and_utc()))
            .map(|dt| Value::String(precision.format(dt)))
            .or_else(|| text(row).map(|s| Value::String(precision.normalize(&s).unwrap_or(s))))
            .unwrap_or(Value::Null),
        ash_core::AttrType::Decimal => get::<rust_decimal::Decimal>(row, column)
            .map(|n| n.to_string())
            .or_else(|| text(row))
            .map(Value::String)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Binary => get::<Vec<u8>>(row, column)
            .map(|bytes| Value::String(ash_core::Binary::from_bytes(bytes).encode()))
            .unwrap_or(Value::Null),
        ash_core::AttrType::Date => get::<chrono::NaiveDate>(row, column)
            .map(|date| date.format("%Y-%m-%d").to_string())
            .or_else(|| text(row))
            .map(Value::String)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Float => get::<f64>(row, column)
            .or_else(|| get::<f32>(row, column).map(f64::from))
            .map(|n| n.to_string())
            .or_else(|| text(row))
            .map(Value::String)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Integer => get::<i64>(row, column)
            .or_else(|| get::<i32>(row, column).map(i64::from))
            .or_else(|| get::<i16>(row, column).map(i64::from))
            .map(Value::Int)
            .unwrap_or(Value::Null),
        ash_core::AttrType::Boolean => get::<bool>(row, column).map(Value::Bool).unwrap_or(Value::Null),
        ash_core::AttrType::Map
        | ash_core::AttrType::Array { .. }
        | ash_core::AttrType::Embedded(_)
        | ash_core::AttrType::TypedMap { .. }
        | ash_core::AttrType::Union { .. } => get::<serde_json::Value>(row, column)
            .or_else(|| text(row).and_then(|s| serde_json::from_str(&s).ok()))
            .map(Value::from_plain_json)
            .unwrap_or(Value::Null),
    }
}

/// The error an atomic statement's condition raised through `ash_raise_error`, if that's
/// what `err` is: `ash_error: {"condition": n, "row": {...}}`.
fn raised_error(err: &Failure, resource: &ResourceDef, conditions: &[ash_core::AtomicCondition]) -> Option<Error> {
    let Failure::Db(err) = err else {
        return None;
    };
    let db_err = err.as_db_error()?;
    let payload: serde_json::Value = serde_json::from_str(db_err.message().strip_prefix("ash_error: ")?).ok()?;
    let condition = conditions.get(payload.get("condition")?.as_u64()? as usize)?;
    let mut row = FieldMap::new();
    if let Some(reported) = payload.get("row").and_then(serde_json::Value::as_object) {
        for (name, value) in reported {
            let ty = resource.attribute(name).map(|attr| attr.ty);
            row.insert(name.clone(), json_value(ty, value));
        }
    }
    Some((condition.error)(&row))
}

/// A record's value as `jsonb_build_object` wrote it, back as the attribute's value.
fn json_value(ty: Option<ash_core::AttrType>, json: &serde_json::Value) -> Value {
    use ash_core::AttrType;
    match (ty, json) {
        (_, serde_json::Value::Null) => Value::Null,
        (Some(AttrType::Uuid), serde_json::Value::String(text)) => {
            Uuid::parse_str(text).map(Value::Uuid).unwrap_or_else(|_| Value::String(text.clone()))
        }
        (_, serde_json::Value::Bool(b)) => Value::Bool(*b),
        (Some(AttrType::Integer), serde_json::Value::Number(n)) => n.as_i64().map(Value::Int).unwrap_or(Value::Null),
        (_, serde_json::Value::Number(n)) => Value::String(n.to_string()),
        (_, serde_json::Value::String(text)) => Value::String(text.clone()),
        (_, other) => Value::String(other.to_string()),
    }
}

/// A driver error, with the database's own message when it has one.
fn map_pg(err: tokio_postgres::Error) -> Error {
    match err.as_db_error() {
        Some(db_err) => Error::DataLayer(db_err.to_string()),
        None => Error::DataLayer(err.to_string()),
    }
}

fn map_failure(failure: Failure) -> Error {
    match failure {
        Failure::Pool(message) => Error::DataLayer(message),
        Failure::Db(err) => map_pg(err),
    }
}

fn map_failure_resource(failure: Failure, resource: &ResourceDef) -> Error {
    let Failure::Db(err) = failure else {
        return map_failure(failure);
    };
    let Some(db_err) = err.as_db_error() else {
        return map_pg(err);
    };
    match db_err.code().code() {
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
            Error::IdentityConflict {
                identity: id_name,
                fields: Vec::new(),
                message: "unique constraint violation".to_string(),
            }
        }
        "23503" => Error::Invalid("foreign key violation: referenced record does not exist".to_string()),
        "23514" => Error::Validation {
            field: "validation".to_string(),
            message: db_err.message().to_string(),
            vars: Vec::new(),
        },
        "40P01" => Error::DataLayer("deadlock detected".to_string()),
        _ => map_pg(err),
    }
}
