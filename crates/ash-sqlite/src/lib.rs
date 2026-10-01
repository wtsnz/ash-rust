//! SQLite data layer. Filters compile to bound SQL instead of scanning HashMaps.

use std::future::Future;
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ash_core::{
    CompiledQuery, DataLayer, Error, FieldMap, ResourceDef, Result, SchemaSupport,
    TransactionSupport, Value,
};
use ash_sql::{
    CompiledSql, MigrationExecutor, Migrator, SqlParam, SqliteDialect, persistable_resources,
};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteQueryResult,
    SqliteRow,
};
use tokio::sync::Mutex;
use uuid::Uuid;

pub mod sql;

pub use sql::create_table_sql;

#[derive(Clone, Debug)]
enum SqliteSource {
    Pool(SqlitePool),
    Tx(Arc<Mutex<sqlx::pool::PoolConnection<sqlx::Sqlite>>>),
}

#[derive(Clone, Debug)]
pub struct Sqlite {
    source: SqliteSource,
}

impl Sqlite {
    pub async fn memory() -> Result<Self> {
        Self::connect("sqlite::memory:").await
    }

    pub async fn connect(url: &str) -> Result<Self> {
        let options = SqliteConnectOptions::from_str(url)
            .map_err(map_sqlx)?
            .create_if_missing(true)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(30));
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;
        Ok(Self {
            source: SqliteSource::Pool(pool),
        })
    }

    pub async fn file(path: impl AsRef<Path>) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(30));
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .map_err(map_sqlx)?;
        Ok(Self {
            source: SqliteSource::Pool(pool),
        })
    }

    pub fn pool(&self) -> Option<&SqlitePool> {
        match &self.source {
            SqliteSource::Pool(p) => Some(p),
            SqliteSource::Tx(_) => None,
        }
    }

    async fn execute_query(&self, compiled: &CompiledSql) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.execute(pool).await.map_err(map_sqlx)
            }
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.execute(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_query_resource(
        &self,
        compiled: &CompiledSql,
        resource: &ResourceDef,
    ) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .execute(pool)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query
                    .execute(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_all(&self, compiled: &CompiledSql) -> Result<Vec<SqliteRow>> {
        match &self.source {
            SqliteSource::Pool(pool) => {
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.fetch_all(pool).await.map_err(map_sqlx)
            }
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                let query = bind_compiled(sqlx::query(&compiled.sql), &compiled.params);
                query.fetch_all(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_raw(&self, sql: &str) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => sqlx::query(sql).execute(pool).await.map_err(map_sqlx),
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                sqlx::query(sql)
                    .execute(&mut **guard)
                    .await
                    .map_err(map_sqlx)
            }
        }
    }

    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        let resources = persistable_resources(resources);
        for resource in &resources {
            let ddl = sql::create_table_sql(resource)?;
            self.execute_raw(&ddl).await?;
            for index_ddl in sql::create_indexes_sql(resource)? {
                self.execute_raw(&index_ddl).await?;
            }
        }
        let has_statements = resources.iter().any(|res| {
            res.statements
                .iter()
                .any(|s| s.dialects.is_empty() || s.dialects.contains(&"sqlite"))
        });
        if has_statements {
            self.execute_raw(ash_sql::install::CREATE_STATEMENTS_TABLE).await?;
        }
        for resource in &resources {
            for statement in resource.statements {
                if !statement.dialects.is_empty() && !statement.dialects.contains(&"sqlite") {
                    continue;
                }
                let table = resource.table_name();
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
        if let Some(pool) = self.pool() {
            sqlx::query("PRAGMA foreign_keys = OFF")
                .execute(pool)
                .await
                .map_err(map_sqlx)?;
        }
        let migrator = Migrator::new(SqliteDialect, migrations_dir);
        let result = migrator.run(self).await;
        if let Some(pool) = self.pool() {
            sqlx::query("PRAGMA foreign_keys = ON")
                .execute(pool)
                .await
                .map_err(map_sqlx)?;
        }
        result
    }

    /// Rollback the latest applied migration.
    pub async fn rollback(&self, migrations_dir: impl AsRef<Path>) -> Result<Option<String>> {
        let migrator = Migrator::new(SqliteDialect, migrations_dir);
        migrator.rollback(self).await
    }
}

fn bind_compiled<'q>(
    mut query: sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>>,
    params: &'q [SqlParam],
) -> sqlx::query::Query<'q, sqlx::Sqlite, sqlx::sqlite::SqliteArguments<'q>> {
    for p in params {
        if p.is_list
            && let Value::Array(items) = &p.value
        {
            let json_array = ash_sql::values_to_json_array(items);
            query = query.bind(json_array);
            continue;
        }
        match &p.value {
            Value::Null => {
                query = query.bind(None::<String>);
            }
            Value::Bool(b) => {
                query = query.bind(if *b { 1i64 } else { 0i64 });
            }
            Value::Int(i) => {
                query = query.bind(*i);
            }
            Value::Uuid(u) => {
                query = query.bind(u.to_string());
            }
            Value::String(s) if p.binary => {
                let bytes = ash_core::Binary::parse(s)
                    .map(ash_core::Binary::into_bytes)
                    .unwrap_or_default();
                query = query.bind(bytes);
            }
            Value::String(s) => {
                query = query.bind(s.as_str());
            }
            Value::Map(_) | Value::Array(_) => {
                query = query.bind(p.value.to_plain_json().to_string());
            }
        }
    }
    query
}

impl Sqlite {
    /// Runs a migration script and its bookkeeping in one `BEGIN IMMEDIATE` transaction
    /// with foreign keys off, if `guard` still returns a row once the write lock is ours.
    /// `binds` go to `bookkeeping`, and the first also to `guard`.
    async fn migration_step(
        &self,
        guard: &str,
        sql: &str,
        bookkeeping: &str,
        binds: &[&str],
    ) -> Result<bool> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        let mut conn = pool.acquire().await.map_err(map_sqlx)?;
        // Table rebuilds drop tables that others reference, so foreign keys stay off while
        // the script runs. The pragma is ignored inside a transaction, so set it first.
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await
            .map_err(map_sqlx)?;
        let outcome = async {
            sqlx::query("BEGIN IMMEDIATE").execute(&mut *conn).await?;
            let wanted = sqlx::query(guard)
                .bind(binds[0])
                .fetch_optional(&mut *conn)
                .await?
                .is_some();
            if !wanted {
                sqlx::query("ROLLBACK").execute(&mut *conn).await?;
                return Ok(false);
            }
            sqlx::raw_sql(sql).execute(&mut *conn).await?;
            let mut query = sqlx::query(bookkeeping);
            for bind in binds {
                query = query.bind(*bind);
            }
            query.execute(&mut *conn).await?;
            sqlx::query("COMMIT").execute(&mut *conn).await?;
            Ok::<bool, sqlx::Error>(true)
        }
        .await;
        if outcome.is_err() {
            // The step's own error is the one to report, so a failed rollback is ignored.
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
        }
        let restored = sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&mut *conn)
            .await;
        let wanted = outcome.map_err(map_sqlx)?;
        restored.map_err(map_sqlx)?;
        Ok(wanted)
    }
}

impl MigrationExecutor for Sqlite {
    async fn execute_script(&self, sql: &str) -> Result<()> {
        let pool = self
            .pool()
            .ok_or_else(|| Error::DataLayer("Migration requires a connection pool".into()))?;
        sqlx::raw_sql(sql).execute(pool).await.map_err(map_sqlx)?;
        Ok(())
    }

    async fn apply_migration(&self, sql: &str, version: &str, name: &str) -> Result<bool> {
        self.migration_step(
            "SELECT 1 WHERE NOT EXISTS (SELECT 1 FROM _ash_schema_migrations WHERE version = ?)",
            sql,
            "INSERT INTO _ash_schema_migrations (version, name, applied_at) VALUES (?, ?, CURRENT_TIMESTAMP)",
            &[version, name],
        )
        .await
    }

    async fn revert_migration(&self, sql: &str, version: &str) -> Result<bool> {
        self.migration_step(
            "SELECT 1 WHERE (SELECT MAX(version) FROM _ash_schema_migrations) = ?",
            sql,
            "DELETE FROM _ash_schema_migrations WHERE version = ?",
            &[version],
        )
        .await
    }

    async fn applied_versions(&self) -> Result<Vec<String>> {
        self.execute_raw(
            "CREATE TABLE IF NOT EXISTS _ash_schema_migrations (
                version TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TEXT NOT NULL
            );",
        )
        .await?;

        let compiled = CompiledSql::new(
            "SELECT version FROM _ash_schema_migrations ORDER BY version ASC".to_string(),
            vec![],
        );
        let rows = self.fetch_all(&compiled).await?;
        let mut versions = Vec::new();
        for row in rows {
            use sqlx::Row;
            let v: String = row.try_get("version").map_err(map_sqlx)?;
            versions.push(v);
        }
        Ok(versions)
    }

    async fn record_migration(&self, version: &str, name: &str) -> Result<()> {
        let compiled = CompiledSql::new(
            "INSERT INTO _ash_schema_migrations (version, name, applied_at) VALUES (?, ?, CURRENT_TIMESTAMP)"
                .to_string(),
            vec![
                SqlParam::new(Value::String(version.to_string())),
                SqlParam::new(Value::String(name.to_string())),
            ],
        );
        self.execute_query(&compiled).await?;
        Ok(())
    }

    async fn remove_migration(&self, version: &str) -> Result<()> {
        let compiled = CompiledSql::new(
            "DELETE FROM _ash_schema_migrations WHERE version = ?".to_string(),
            vec![SqlParam::new(Value::String(version.to_string()))],
        );
        self.execute_query(&compiled).await?;
        Ok(())
    }
}

impl SchemaSupport for Sqlite {
    async fn install_resources(&self, resources: &[&ResourceDef]) -> Result<()> {
        self.install(resources).await
    }
}

impl DataLayer for Sqlite {
    async fn create(
        &self,
        resource: &ResourceDef,
        _id: Uuid,
        fields: FieldMap,
    ) -> Result<FieldMap> {
        let qb = sql::insert_query(resource, &fields)?;
        self.execute_query_resource(&qb, resource).await?;
        Ok(fields)
    }

    async fn update(&self, resource: &ResourceDef, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        let qb = sql::update_query(resource, id, &fields)?;
        let result = self.execute_query_resource(&qb, resource).await?;
        if result.rows_affected() == 0 {
            if resource.optimistic_lock_attribute().is_some() {
                let pk = resource
                    .primary_key()
                    .ok_or(Error::NoPrimaryKey(resource.name))?;
                let check_sql = format!(
                    "SELECT 1 FROM \"{}\" WHERE \"{}\" = ?",
                    resource.table_name(),
                    pk.name
                );
                let check_compiled =
                    CompiledSql::new(check_sql, vec![SqlParam::new(Value::Uuid(id))]);
                if self.fetch_all(&check_compiled).await?.is_empty() {
                    return Err(Error::NotFound);
                } else {
                    return Err(Error::StaleRecord {
                        resource: resource.name,
                        id,
                    });
                }
            } else {
                return Err(Error::NotFound);
            }
        }

        let pk = resource
            .primary_key()
            .ok_or(Error::NoPrimaryKey(resource.name))?;
        let fetch_sql = format!(
            "SELECT * FROM \"{}\" WHERE \"{}\" = ?",
            resource.table_name(),
            pk.name
        );
        let fetch_compiled = CompiledSql::new(fetch_sql, vec![SqlParam::new(Value::Uuid(id))]);
        let mut fetched = self.fetch_all(&fetch_compiled).await?;
        if let Some(row) = fetched.pop() {
            sql::row_to_fields(&row, resource, &[], &[])
        } else {
            Ok(fields)
        }
    }

    async fn destroy(&self, resource: &ResourceDef, id: Uuid) -> Result<()> {
        let qb = sql::delete_query(resource, id)?;
        let result = self.execute_query(&qb).await?;
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
        let qb = sql::select_query(resource, query)?;
        let rows = self.fetch_all(&qb).await?;
        rows.iter()
            .map(|row| sql::row_to_fields(row, resource, &query.calculations, &query.aggregates))
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
        let qb = sql::upsert_query(resource, &fields, identity, update_fields)?;
        self.execute_query_resource(&qb, resource).await?;

        let mut where_parts = Vec::new();
        let mut params = Vec::new();
        for key in identity.keys {
            if let Some(val) = fields.get(*key) {
                where_parts.push(format!("\"{}\" = ?", key));
                params.push(SqlParam::new(val.clone()));
            } else {
                where_parts.push(format!("\"{}\" IS NULL", key));
            }
        }
        // Rows outside a partial identity never conflict, so the result is either the
        // row we just inserted or the one inside the predicate that absorbed the write.
        let mut order = String::new();
        if let Some(predicate) = identity.predicate {
            let pk = resource
                .primary_key()
                .ok_or(Error::NoPrimaryKey(resource.name))?;
            let pk_value = fields.get(pk.name).cloned().unwrap_or(Value::Null);
            where_parts.push(format!("(\"{}\" = ? OR ({predicate}))", pk.name));
            params.push(SqlParam::new(pk_value.clone()));
            order = format!(" ORDER BY \"{}\" = ? DESC LIMIT 1", pk.name);
            params.push(SqlParam::new(pk_value));
        }
        let fetch_sql = format!(
            "SELECT * FROM \"{}\" WHERE {}{order}",
            resource.table_name(),
            where_parts.join(" AND ")
        );
        let fetch_compiled = CompiledSql::new(fetch_sql, params);
        let rows = self.fetch_all(&fetch_compiled).await?;
        let row = rows.first().ok_or(Error::NotFound)?;
        sql::row_to_fields(row, resource, &[], &[])
    }

    async fn bulk_create(
        &self,
        resource: &ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        if rows.is_empty() {
            return Ok(Vec::new());
        }
        self.transaction(|tx| {
            let tx = tx.clone();
            async move {
                let mut results = Vec::with_capacity(rows.len());
                for (id, fields) in rows {
                    results.push(tx.create(resource, id, fields).await?);
                }
                Ok(results)
            }
        })
        .await
    }

    async fn bulk_destroy(&self, resource: &ResourceDef, ids: &[Uuid]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let qb = sql::bulk_delete_query(resource, ids)?;
        self.execute_query_resource(&qb, resource).await?;
        Ok(())
    }
}

impl TransactionSupport for Sqlite {
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        match &self.source {
            SqliteSource::Pool(pool) => {
                let conn = pool.acquire().await.map_err(map_sqlx)?;
                let conn = Arc::new(Mutex::new(conn));
                {
                    let mut guard = conn.lock().await;
                    sqlx::query("BEGIN")
                        .execute(&mut **guard)
                        .await
                        .map_err(map_sqlx)?;
                }
                let tx_sqlite = Sqlite {
                    source: SqliteSource::Tx(Arc::clone(&conn)),
                };
                match f(&tx_sqlite).await {
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
            SqliteSource::Tx(conn) => {
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

fn map_sqlx(err: sqlx::Error) -> Error {
    match &err {
        sqlx::Error::RowNotFound => Error::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            Error::DataLayer("duplicate id".into())
        }
        _ => Error::DataLayer(err.to_string()),
    }
}

fn map_sqlx_resource(err: sqlx::Error, resource: &ResourceDef) -> Error {
    match &err {
        sqlx::Error::RowNotFound => Error::NotFound,
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            let msg = db.message();
            for ident in resource.identities {
                let matches_keys = ident.keys.iter().all(|k| msg.contains(k));
                let matches_index = msg.contains(ident.name);
                if matches_keys || matches_index {
                    return Error::IdentityConflict {
                        identity: ident.name,
                        fields: ident.keys.iter().map(|s| s.to_string()).collect(),
                        message: ident
                            .message
                            .unwrap_or("record with this identity already exists")
                            .to_string(),
                    };
                }
            }
            Error::DataLayer("duplicate id".into())
        }
        _ => Error::DataLayer(err.to_string()),
    }
}
