//! SQLite data layer. Filters compile to bound SQL instead of scanning HashMaps.

use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use ash_core::{
    CompiledQuery, DataLayer, Error, FieldMap, ResourceDef, Result, SchemaSupport,
    TransactionSupport,
};
use sqlx::sqlite::{
    SqliteConnectOptions, SqliteJournalMode, SqlitePool, SqlitePoolOptions, SqliteQueryResult,
    SqliteRow,
};
use tokio::sync::Mutex;
use uuid::Uuid;

mod sql;

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
            .create_if_missing(true);
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
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(5));
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

    async fn execute_query<'a>(
        &self,
        mut qb: sqlx::QueryBuilder<'a, sqlx::Sqlite>,
    ) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => qb.build().execute(pool).await.map_err(map_sqlx),
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                qb.build().execute(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_query_resource<'a>(
        &self,
        mut qb: sqlx::QueryBuilder<'a, sqlx::Sqlite>,
        resource: &ResourceDef,
    ) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => qb
                .build()
                .execute(pool)
                .await
                .map_err(|e| map_sqlx_resource(e, resource)),
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                qb.build()
                    .execute(&mut **guard)
                    .await
                    .map_err(|e| map_sqlx_resource(e, resource))
            }
        }
    }

    async fn fetch_all<'a>(
        &self,
        mut qb: sqlx::QueryBuilder<'a, sqlx::Sqlite>,
    ) -> Result<Vec<SqliteRow>> {
        match &self.source {
            SqliteSource::Pool(pool) => qb.build().fetch_all(pool).await.map_err(map_sqlx),
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                qb.build().fetch_all(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    async fn execute_raw(&self, sql: &str) -> Result<SqliteQueryResult> {
        match &self.source {
            SqliteSource::Pool(pool) => sqlx::query(sql).execute(pool).await.map_err(map_sqlx),
            SqliteSource::Tx(conn) => {
                let mut guard = conn.lock().await;
                sqlx::query(sql).execute(&mut **guard).await.map_err(map_sqlx)
            }
        }
    }

    pub async fn install(&self, resources: &[&ResourceDef]) -> Result<()> {
        for resource in resources {
            let ddl = sql::create_table_sql(resource)?;
            self.execute_raw(&ddl).await?;
            for index_ddl in sql::create_indexes_sql(resource)? {
                self.execute_raw(&index_ddl).await?;
            }
        }
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
        self.execute_query_resource(qb, resource).await?;
        Ok(fields)
    }

    async fn update(&self, resource: &ResourceDef, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        let qb = sql::update_query(resource, id, &fields)?;
        let result = self.execute_query_resource(qb, resource).await?;
        if result.rows_affected() == 0 {
            if resource.optimistic_lock_attribute().is_some() {
                let pk = resource
                    .primary_key()
                    .ok_or(Error::NoPrimaryKey(resource.name))?;
                let mut check_qb = sqlx::QueryBuilder::new("SELECT 1 FROM ");
                check_qb.push(sql::ident(resource.table_name())?);
                check_qb.push(" WHERE ");
                check_qb.push(sql::ident(pk.name)?);
                check_qb.push(" = ");
                check_qb.push_bind(id.to_string());
                if self.fetch_all(check_qb).await?.is_empty() {
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
        let mut fetch_qb = sqlx::QueryBuilder::new("SELECT * FROM ");
        fetch_qb.push(sql::ident(resource.table_name())?);
        fetch_qb.push(" WHERE ");
        fetch_qb.push(sql::ident(pk.name)?);
        fetch_qb.push(" = ");
        fetch_qb.push_bind(id.to_string());
        let mut fetched = self.fetch_all(fetch_qb).await?;
        if let Some(row) = fetched.pop() {
            sql::row_to_fields(&row, resource, &[], &[])
        } else {
            Ok(fields)
        }
    }

    async fn destroy(&self, resource: &ResourceDef, id: Uuid) -> Result<()> {
        let qb = sql::delete_query(resource, id)?;
        let result = self.execute_query(qb).await?;
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
        let rows = self.fetch_all(qb).await?;
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
        self.execute_query_resource(qb, resource).await?;

        let mut fetch_qb = sqlx::QueryBuilder::new("SELECT * FROM ");
        fetch_qb.push(sql::ident(resource.table_name())?);
        fetch_qb.push(" WHERE ");
        for (i, key) in identity.keys.iter().enumerate() {
            if i > 0 {
                fetch_qb.push(" AND ");
            }
            fetch_qb.push(sql::ident(key)?);
            fetch_qb.push(" = ");
            if let Some(val) = fields.get(*key) {
                sql::push_sql_value(&mut fetch_qb, val);
            } else {
                fetch_qb.push("NULL");
            }
        }
        let rows = self.fetch_all(fetch_qb).await?;
        let row = rows.first().ok_or(Error::NotFound)?;
        sql::row_to_fields(row, resource, &[], &[])
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
