use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use ash_core::ResourceDef;
use cargo_ash::codegen::{
    self, CodegenError, CodegenOptions, CodegenOutcome, Dialect, Mode, NonInteractive,
    RenameResolver, WrittenMigration,
};
use sqlx::Row;
use uuid::Uuid;

/// Runs a scenario once against SQLite and once against Postgres. The Postgres run is skipped
/// only when `DATABASE_URL` is unset; an unreachable database fails the test.
macro_rules! on_every_backend {
    ($scenario:ident) => {
        mod $scenario {
            #[tokio::test]
            async fn sqlite() {
                super::$scenario(crate::support::TestDb::sqlite().await).await;
            }

            #[tokio::test]
            async fn postgres() {
                if let Some(db) = crate::support::TestDb::postgres().await {
                    super::$scenario(db).await;
                }
            }
        }
    };
}
pub(crate) use on_every_backend;

pub fn scratch_dir(kind: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ash_{kind}_{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub enum Db {
    Sqlite(ash_sqlite::Sqlite),
    Postgres(ash_postgres::Postgres),
}

pub struct TestDb {
    pub db: Db,
    pub url: String,
}

impl TestDb {
    pub async fn sqlite() -> Self {
        let path = scratch_dir("db").join("test.db");
        Self::connect(format!("sqlite://{}", path.display())).await
    }

    pub async fn postgres() -> Option<Self> {
        let Ok(base) = std::env::var("DATABASE_URL") else {
            eprintln!("skipping Postgres scenario: DATABASE_URL is not set");
            return None;
        };
        let admin = ash_postgres::Postgres::connect(&base)
            .await
            .expect("DATABASE_URL is set but Postgres is unreachable");
        let schema = format!("codegen_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
            .execute(admin.pool().unwrap())
            .await
            .unwrap();
        let separator = if base.contains('?') { '&' } else { '?' };
        Some(
            Self::connect(format!(
                "{base}{separator}options=-c%20search_path%3D{schema}"
            ))
            .await,
        )
    }

    async fn connect(url: String) -> Self {
        let db = if url.starts_with("sqlite:") {
            Db::Sqlite(ash_sqlite::Sqlite::connect(&url).await.unwrap())
        } else {
            Db::Postgres(ash_postgres::Postgres::connect(&url).await.unwrap())
        };
        Self { db, url }
    }

    /// An independent connection to the same database, as a second app instance would have.
    pub async fn reconnect(&self) -> Self {
        Self::connect(self.url.clone()).await
    }

    pub fn dialect(&self) -> Dialect {
        match self.db {
            Db::Sqlite(_) => Dialect::Sqlite,
            Db::Postgres(_) => Dialect::Postgres,
        }
    }

    pub fn text_type(&self) -> &'static str {
        match self.db {
            Db::Sqlite(_) => "TEXT",
            Db::Postgres(_) => "text",
        }
    }

    pub fn integer_type(&self) -> &'static str {
        match self.db {
            Db::Sqlite(_) => "INTEGER",
            Db::Postgres(_) => "bigint",
        }
    }

    pub fn uuid_type(&self) -> &'static str {
        match self.db {
            Db::Sqlite(_) => "TEXT",
            Db::Postgres(_) => "uuid",
        }
    }

    pub fn datetime_type(&self) -> &'static str {
        match self.db {
            Db::Sqlite(_) => "TEXT",
            Db::Postgres(_) => "timestamp with time zone",
        }
    }

    pub fn decimal_type(&self) -> &'static str {
        match self.db {
            Db::Sqlite(_) => "NUMERIC",
            Db::Postgres(_) => "numeric",
        }
    }

    pub async fn migrate(&self, dir: &Path) -> ash_core::Result<Vec<String>> {
        match &self.db {
            Db::Sqlite(db) => db.migrate(dir).await,
            Db::Postgres(db) => db.migrate(dir).await,
        }
    }

    pub async fn rollback(&self, dir: &Path) -> ash_core::Result<Option<String>> {
        match &self.db {
            Db::Sqlite(db) => db.rollback(dir).await,
            Db::Postgres(db) => db.rollback(dir).await,
        }
    }

    pub async fn install(&self, resources: &[&ResourceDef]) -> ash_core::Result<()> {
        match &self.db {
            Db::Sqlite(db) => db.install(resources).await,
            Db::Postgres(db) => db.install(resources).await,
        }
    }

    pub async fn exec(&self, sql: &str) -> Result<(), sqlx::Error> {
        match &self.db {
            Db::Sqlite(db) => sqlx::query(sql)
                .execute(db.pool().unwrap())
                .await
                .map(|_| ()),
            Db::Postgres(db) => sqlx::query(sql)
                .execute(db.pool().unwrap())
                .await
                .map(|_| ()),
        }
    }

    pub async fn int(&self, sql: &str) -> i64 {
        match &self.db {
            Db::Sqlite(db) => sqlx::query_scalar(sql)
                .persistent(false)
                .fetch_one(db.pool().unwrap())
                .await
                .unwrap(),
            Db::Postgres(db) => sqlx::query_scalar(sql)
                .persistent(false)
                .fetch_one(db.pool().unwrap())
                .await
                .unwrap(),
        }
    }

    pub async fn text(&self, sql: &str) -> String {
        match &self.db {
            Db::Sqlite(db) => sqlx::query_scalar(sql)
                .persistent(false)
                .fetch_one(db.pool().unwrap())
                .await
                .unwrap(),
            Db::Postgres(db) => sqlx::query_scalar(sql)
                .persistent(false)
                .fetch_one(db.pool().unwrap())
                .await
                .unwrap(),
        }
    }

    pub async fn applied_versions(&self) -> Vec<String> {
        let sql = "SELECT version FROM _ash_schema_migrations ORDER BY version";
        match &self.db {
            Db::Sqlite(db) => sqlx::query_scalar(sql).fetch_all(db.pool().unwrap()).await,
            Db::Postgres(db) => sqlx::query_scalar(sql).fetch_all(db.pool().unwrap()).await,
        }
        .unwrap_or_default()
    }

    pub async fn schema(&self) -> DbSchema {
        match &self.db {
            Db::Sqlite(db) => sqlite_schema(db.pool().unwrap()).await,
            Db::Postgres(db) => postgres_schema(db.pool().unwrap()).await,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DbSchema {
    pub tables: BTreeMap<String, Table>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    pub columns: BTreeMap<String, Column>,
    pub unique_indexes: BTreeMap<String, Vec<String>>,
    pub indexes: BTreeMap<String, Vec<String>>,
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Column {
    pub ty: String,
    pub nullable: bool,
    pub default: Option<String>,
    pub primary_key: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForeignKey {
    pub column: String,
    pub references_table: String,
    pub references_column: String,
    pub on_delete: String,
}

impl DbSchema {
    pub fn table_names(&self) -> Vec<&str> {
        self.tables.keys().map(String::as_str).collect()
    }

    pub fn table(&self, name: &str) -> &Table {
        self.tables
            .get(name)
            .unwrap_or_else(|| panic!("table `{name}` missing from {:?}", self.table_names()))
    }

    pub fn column(&self, table: &str, column: &str) -> &Column {
        self.table(table)
            .columns
            .get(column)
            .unwrap_or_else(|| panic!("column `{table}.{column}` missing"))
    }

    pub fn has_column(&self, table: &str, column: &str) -> bool {
        self.table(table).columns.contains_key(column)
    }
}

const TRACKING_TABLE: &str = "_ash_schema_migrations";

async fn sqlite_schema(pool: &sqlx::SqlitePool) -> DbSchema {
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
    )
    .fetch_all(pool)
    .await
    .unwrap();

    let mut schema = DbSchema::default();
    for name in names.into_iter().filter(|n| n != TRACKING_TABLE) {
        let mut table = Table::default();

        for row in sqlx::query(&format!("PRAGMA table_info(\"{name}\")"))
            .fetch_all(pool)
            .await
            .unwrap()
        {
            let column: String = row.get("name");
            let not_null: i64 = row.get("notnull");
            let pk: i64 = row.get("pk");
            table.columns.insert(
                column,
                Column {
                    ty: row.get("type"),
                    nullable: not_null == 0,
                    default: row.get("dflt_value"),
                    primary_key: pk > 0,
                },
            );
        }

        for row in sqlx::query(&format!("PRAGMA index_list(\"{name}\")"))
            .fetch_all(pool)
            .await
            .unwrap()
        {
            let unique: i64 = row.get("unique");
            let origin: String = row.get("origin");
            if origin == "pk" {
                continue;
            }
            let index: String = row.get("name");
            let columns: Vec<String> = sqlx::query(&format!("PRAGMA index_info(\"{index}\")"))
                .fetch_all(pool)
                .await
                .unwrap()
                .iter()
                .map(|r| r.get("name"))
                .collect();
            if unique == 0 {
                table.indexes.insert(index, columns);
            } else {
                table.unique_indexes.insert(index, columns);
            }
        }

        for row in sqlx::query(&format!("PRAGMA foreign_key_list(\"{name}\")"))
            .fetch_all(pool)
            .await
            .unwrap()
        {
            table.foreign_keys.push(ForeignKey {
                column: row.get("from"),
                references_table: row.get("table"),
                references_column: row.get("to"),
                on_delete: row.get("on_delete"),
            });
        }
        table.foreign_keys.sort();

        schema.tables.insert(name, table);
    }
    schema
}

async fn postgres_schema(pool: &sqlx::PgPool) -> DbSchema {
    let mut schema = DbSchema::default();

    let columns = sqlx::query(
        "SELECT c.table_name::text AS table_name, c.column_name::text AS column_name,
                c.data_type::text AS data_type, c.is_nullable::text AS is_nullable,
                c.column_default::text AS column_default,
                EXISTS (
                    SELECT 1 FROM information_schema.table_constraints tc
                    JOIN information_schema.key_column_usage k
                      ON k.constraint_name = tc.constraint_name AND k.constraint_schema = tc.constraint_schema
                    WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = c.table_schema
                      AND tc.table_name = c.table_name AND k.column_name = c.column_name
                ) AS primary_key
         FROM information_schema.columns c
         JOIN information_schema.tables t
           ON t.table_schema = c.table_schema AND t.table_name = c.table_name AND t.table_type = 'BASE TABLE'
         WHERE c.table_schema = current_schema()",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for row in columns {
        let table: String = row.get("table_name");
        if table == TRACKING_TABLE {
            continue;
        }
        let default: Option<String> = row.get("column_default");
        let is_nullable: String = row.get("is_nullable");
        schema.tables.entry(table).or_default().columns.insert(
            row.get("column_name"),
            Column {
                ty: row.get("data_type"),
                nullable: is_nullable == "YES",
                default: default.map(strip_postgres_cast),
                primary_key: row.get("primary_key"),
            },
        );
    }

    let indexes = sqlx::query(
        "SELECT t.relname::text AS table_name, i.relname::text AS index_name,
                array_agg(a.attname::text ORDER BY k.ord) AS columns
         FROM pg_index ix
         JOIN pg_class t ON t.oid = ix.indrelid
         JOIN pg_class i ON i.oid = ix.indexrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord)
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
         WHERE n.nspname = current_schema() AND ix.indisunique AND NOT ix.indisprimary
         GROUP BY t.relname, i.relname",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for row in indexes {
        let table: String = row.get("table_name");
        if let Some(t) = schema.tables.get_mut(&table) {
            t.unique_indexes
                .insert(row.get("index_name"), row.get("columns"));
        }
    }

    let non_unique = sqlx::query(
        "SELECT t.relname::text AS table_name, i.relname::text AS index_name,
                array_agg(a.attname::text ORDER BY k.ord) AS columns
         FROM pg_index ix
         JOIN pg_class t ON t.oid = ix.indrelid
         JOIN pg_class i ON i.oid = ix.indexrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         CROSS JOIN LATERAL unnest(ix.indkey) WITH ORDINALITY AS k(attnum, ord)
         JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = k.attnum
         WHERE n.nspname = current_schema() AND NOT ix.indisunique AND NOT ix.indisprimary
         GROUP BY t.relname, i.relname",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for row in non_unique {
        let table: String = row.get("table_name");
        if let Some(t) = schema.tables.get_mut(&table) {
            t.indexes
                .insert(row.get("index_name"), row.get("columns"));
        }
    }

    let foreign_keys = sqlx::query(
        "SELECT cl.relname::text AS table_name, a.attname::text AS column_name,
                rt.relname::text AS references_table, ra.attname::text AS references_column,
                c.confdeltype::text AS on_delete
         FROM pg_constraint c
         JOIN pg_class cl ON cl.oid = c.conrelid
         JOIN pg_namespace n ON n.oid = cl.relnamespace
         JOIN pg_class rt ON rt.oid = c.confrelid
         JOIN pg_attribute a ON a.attrelid = c.conrelid AND a.attnum = c.conkey[1]
         JOIN pg_attribute ra ON ra.attrelid = c.confrelid AND ra.attnum = c.confkey[1]
         WHERE c.contype = 'f' AND n.nspname = current_schema()",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    for row in foreign_keys {
        let table: String = row.get("table_name");
        let code: String = row.get("on_delete");
        let on_delete = match code.as_str() {
            "c" => "CASCADE",
            "n" => "SET NULL",
            "r" => "RESTRICT",
            "d" => "SET DEFAULT",
            _ => "NO ACTION",
        };
        if let Some(t) = schema.tables.get_mut(&table) {
            t.foreign_keys.push(ForeignKey {
                column: row.get("column_name"),
                references_table: row.get("references_table"),
                references_column: row.get("references_column"),
                on_delete: on_delete.to_string(),
            });
            t.foreign_keys.sort();
        }
    }

    schema
}

fn strip_postgres_cast(default: String) -> String {
    match default.rfind("::") {
        Some(at) if default[..at].ends_with('\'') => default[..at].to_string(),
        _ => default,
    }
}

pub struct Project {
    pub root: PathBuf,
    pub dialect: Dialect,
}

impl Project {
    pub fn new(dialect: Dialect) -> Self {
        Self {
            root: scratch_dir("project"),
            dialect,
        }
    }

    pub fn for_db(db: &TestDb) -> Self {
        Self::new(db.dialect())
    }

    pub fn migrations(&self) -> PathBuf {
        self.root.join("migrations")
    }

    pub fn snapshots(&self) -> PathBuf {
        self.root.join("resource_snapshots")
    }

    pub fn options(&self, mode: Mode, name: Option<&str>) -> CodegenOptions {
        CodegenOptions {
            name: name.map(str::to_string),
            migrations_dir: self.migrations(),
            snapshots_dir: self.snapshots(),
            mode,
            ..CodegenOptions::new(self.dialect)
        }
    }

    pub fn run(
        &self,
        options: &CodegenOptions,
        resources: &[&'static ResourceDef],
        resolver: &mut dyn RenameResolver,
    ) -> Result<CodegenOutcome, CodegenError> {
        codegen::run(resources, options, resolver)
    }

    pub fn generate(&self, name: &str, resources: &[&'static ResourceDef]) -> WrittenMigration {
        let options = self.options(Mode::Write, Some(name));
        match self.run(&options, resources, &mut NonInteractive) {
            Ok(CodegenOutcome::Written(migration)) => migration,
            other => panic!("expected `{name}` to write a migration, got {other:?}"),
        }
    }

    pub fn migration_files(&self) -> Vec<String> {
        list_files(&self.migrations())
    }

    pub fn snapshot_files(&self) -> Vec<String> {
        list_files(&self.snapshots().join(self.dialect.name()))
    }
}

fn list_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}
