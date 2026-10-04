#![allow(dead_code)]

use std::collections::BTreeMap;

use ash_core::{Context, DataLayer, TransactionSupport};
use ash_mailer::MemoryMailer;
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use lagrange::Lagrange;
use lagrange::seed::{self, Sol};

/// What a fleet database must support to run the workflows.
pub trait FleetDb: DataLayer + TransactionSupport + Clone + Send + Sync + 'static {}
impl<T: DataLayer + TransactionSupport + Clone + Send + Sync + 'static> FleetDb for T {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Memory,
    Sqlite,
    Postgres,
}

/// A seeded platform and signed-in contexts for every crew member.
pub struct Harness<D> {
    pub backend: Backend,
    pub app: Lagrange<D>,
    pub mailer: MemoryMailer,
    pub sol: Sol,
    ctx: BTreeMap<&'static str, Context<D>>,
    /// Keeps a SQLite database file alive for the test.
    _dir: Option<tempfile::TempDir>,
}

impl<D: FleetDb> Harness<D> {
    async fn seed(backend: Backend, fleet: D, dir: Option<tempfile::TempDir>) -> Self {
        let mailer = MemoryMailer::new();
        let app = Lagrange::new(fleet, telemetry().await, mailer.clone());
        let sol = seed::sol(&app).await.expect("seed the Sol system");
        let ctx = seed::contexts(&app, &sol);
        Self {
            backend,
            app,
            mailer,
            sol,
            ctx,
            _dir: dir,
        }
    }

    /// The signed-in context of a seeded crew member, by first name.
    pub fn ctx(&self, name: &str) -> &Context<D> {
        &self.ctx[name]
    }

    /// The customs officer, inspecting cargo of `line`.
    pub fn customs(&self, line: &str) -> Context<D> {
        self.ctx["Chen"].clone().with_tenant(line)
    }
}

/// A telemetry database, created in memory.
pub async fn telemetry() -> Sqlite {
    let telemetry = Sqlite::memory().await.unwrap();
    lagrange::Telemetry::new(telemetry.clone())
        .install()
        .await
        .unwrap();
    telemetry
}

pub async fn memory() -> Harness<Memory> {
    Harness::seed(Backend::Memory, Memory::new(), None).await
}

/// A SQLite file migrated with the committed migrations.
pub async fn sqlite() -> Harness<Sqlite> {
    let dir = tempfile::tempdir().unwrap();
    let fleet = Sqlite::file(dir.path().join("fleet.db")).await.unwrap();
    fleet.migrate(lagrange::migrations_dir()).await.unwrap();
    Harness::seed(Backend::Sqlite, fleet, Some(dir)).await
}

/// A fresh schema in the `DATABASE_URL` database, migrated with the committed
/// migrations. `None` when `DATABASE_URL` is unset, which CI never allows.
pub async fn postgres_fleet() -> Option<Postgres> {
    let Ok(base) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var("CI").is_err(), "CI must set DATABASE_URL");
        eprintln!("skipping Postgres: DATABASE_URL is not set");
        return None;
    };
    // Setup SQL runs through a pool of its own: the data layer's is tokio-postgres's.
    let admin = sqlx::PgPool::connect(&base).await.expect("connect to DATABASE_URL");
    // Extensions are database-wide and racing `CREATE EXTENSION` calls collide, so
    // install them once, under a lock, before any schema migrates.
    let mut tx = admin.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(7303013)")
        .execute(&mut *tx)
        .await
        .unwrap();
    for extension in ["citext", "vector", "btree_gist"] {
        sqlx::query(&format!(
            "CREATE EXTENSION IF NOT EXISTS {extension} WITH SCHEMA public"
        ))
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let schema = format!("lagrange_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA \"{schema}\""))
        .execute(&admin)
        .await
        .unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let fleet = Postgres::connect(&format!(
        "{base}{separator}options=-c%20search_path%3D{schema}%2Cpublic"
    ))
    .await
    .unwrap();
    fleet.migrate(lagrange::migrations_dir()).await.unwrap();
    Some(fleet)
}

pub async fn postgres() -> Option<Harness<Postgres>> {
    let fleet = postgres_fleet().await?;
    Some(Harness::seed(Backend::Postgres, fleet, None).await)
}

/// Runs `scenario` on memory, SQLite, and Postgres.
#[macro_export]
macro_rules! on_every_backend {
    ($scenario:ident) => {
        mod $scenario {
            #[tokio::test(flavor = "multi_thread")]
            async fn memory() {
                super::$scenario($crate::support::memory().await).await;
            }

            #[tokio::test(flavor = "multi_thread")]
            async fn sqlite() {
                super::$scenario($crate::support::sqlite().await).await;
            }

            #[tokio::test(flavor = "multi_thread")]
            async fn postgres() {
                if let Some(harness) = $crate::support::postgres().await {
                    super::$scenario(harness).await;
                }
            }
        }
    };
}
