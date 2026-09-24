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
    CompiledQuery, DataLayer, Error, FieldMap, ResourceDef, Result, SchemaSupport,
    TransactionSupport, Value,
};
use ash_sql::{CompiledSql, MigrationExecutor, Migrator, PostgresDialect, QueryCompiler, SqlParam};
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
    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        let resources = ash_sql::persistable_resources(resources);
        let dialect = PostgresDialect;
        let compiler = QueryCompiler::new(&dialect);
        for res in resources {
            let ddl = compiler.compile_create_table(res)?;
            self.execute_raw(&ddl).await?;
            for idx_ddl in compiler.compile_create_indexes(res)? {
                self.execute_raw(&idx_ddl).await?;
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
            let opts = (*pool.connect_options())
                .clone()
                .options([("search_path", *name)]);
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
        _id: Uuid,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_insert(resource, &fields)?;

        // Postgres supports RETURNING * for single-roundtrip writes!
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource, &[], &[])
    }

    async fn update(&self, resource: &ResourceDef, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
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
                    let check_sql = format!(
                        "SELECT 1 FROM \"{}\" WHERE \"{}\" = $1",
                        resource.table_name(),
                        pk.name
                    );
                    let check_compiled =
                        CompiledSql::new(check_sql, vec![SqlParam::new(Value::Uuid(id))]);
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

    async fn destroy(&self, resource: &ResourceDef, id: Uuid) -> Result<()> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
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
        // Schema multitenancy: if tenant is specified, apply search_path
        if let Some(tenant) = &query.tenant {
            let set_search_path = format!(
                "SET LOCAL search_path TO \"{}\", \"public\"",
                tenant.replace('"', "\"\"")
            );
            let _ = self.execute_raw(&set_search_path).await;
        }

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
        _id: Uuid,
        fields: FieldMap,
        identity: &ash_core::IdentityDef,
        update_fields: &[String],
    ) -> Result<FieldMap> {
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_upsert(resource, &fields, identity, update_fields)?;

        // Single-roundtrip write with RETURNING *
        let row = self.fetch_one_resource(&compiled, resource).await?;
        row_to_fields(&row, resource, &[], &[])
    }

    async fn bulk_create(
        &self,
        resource: &ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
        let compiled = compiler.compile_bulk_insert(resource, &rows)?;
        let pg_rows = self.fetch_all_resource(&compiled, resource).await?;
        pg_rows
            .iter()
            .map(|r| row_to_fields(r, resource, &[], &[]))
            .collect()
    }

    async fn bulk_destroy(&self, resource: &ResourceDef, ids: &[Uuid]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let dialect = PostgresDialect;
        let mut compiler = QueryCompiler::new(&dialect);
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
        sqlx::query(self.create_table_sql)
            .execute(&self.pool)
            .await
            .map_err(map_sqlx)?;
        Ok(())
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

    async fn apply_migration(&self, sql: &str, version: &str, name: &str) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(map_sqlx)?;
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
        Ok(())
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
            Value::Null => {
                query = query.bind(None::<String>);
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
            Value::Map(m) => {
                let json = serde_json::to_string(m).unwrap_or_else(|_| "{}".to_string());
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                query = query.bind(parsed);
            }
            Value::Array(a) => {
                let json = serde_json::to_string(a).unwrap_or_else(|_| "[]".to_string());
                let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
                query = query.bind(parsed);
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

fn extract_column_value(row: &PgRow, col_name: &str, ty: &ash_core::AttrType) -> Value {
    match ty {
        ash_core::AttrType::Uuid => {
            if let Ok(Some(u)) = row.try_get::<Option<Uuid>, _>(col_name) {
                Value::Uuid(u)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Uuid::parse_str(&s).map(Value::Uuid).unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::String | ash_core::AttrType::Atom { .. } => {
            if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::UtcDatetime => {
            if let Ok(Some(dt)) = row.try_get::<Option<chrono::DateTime<chrono::Utc>>, _>(col_name)
            {
                Value::String(format_utc(dt))
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                Value::String(s)
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
                json_to_ash_value(json_val)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                serde_json::from_str::<serde_json::Value>(&s)
                    .map(json_to_ash_value)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
        ash_core::AttrType::Array => {
            if let Ok(Some(json_val)) = row.try_get::<Option<serde_json::Value>, _>(col_name) {
                json_to_ash_value(json_val)
            } else if let Ok(Some(s)) = row.try_get::<Option<String>, _>(col_name) {
                serde_json::from_str::<serde_json::Value>(&s)
                    .map(json_to_ash_value)
                    .unwrap_or(Value::Null)
            } else {
                Value::Null
            }
        }
    }
}

fn format_utc(dt: chrono::DateTime<chrono::Utc>) -> String {
    let format = if dt.timestamp_subsec_nanos() == 0 {
        chrono::SecondsFormat::Secs
    } else {
        chrono::SecondsFormat::AutoSi
    };
    dt.to_rfc3339_opts(format, true)
}

fn json_to_ash_value(val: serde_json::Value) -> Value {
    match val {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => {
            if let Ok(u) = Uuid::parse_str(&s) {
                Value::Uuid(u)
            } else {
                Value::String(s)
            }
        }
        serde_json::Value::Array(a) => {
            let arr = a.into_iter().map(json_to_ash_value).collect();
            Value::Array(arr)
        }
        serde_json::Value::Object(o) => {
            let mut map = FieldMap::new();
            for (k, v) in o {
                map.insert(k, json_to_ash_value(v));
            }
            Value::Map(map)
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
