//! The committed migration history: it matches the resources, rolls all the way back,
//! and its table rename keeps the rows that were there.

use std::path::Path;

use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use cargo_ash::codegen::{self, CodegenOptions, CodegenOutcome, Dialect, Mode, NonInteractive};

fn resources() -> Vec<&'static ash_core::ResourceDef> {
    [&lagrange::WORLD_DEF, &lagrange::FLEET_DEF, &lagrange::CARGO_DEF]
        .into_iter()
        .flat_map(|domain| domain.resources.iter().copied())
        .collect()
}

#[test]
fn migrations_match_the_resources() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    for dialect in [Dialect::Sqlite, Dialect::Postgres] {
        let options = CodegenOptions {
            migrations_dir: root.join("migrations"),
            snapshots_dir: root.join("resource_snapshots"),
            mode: Mode::Check,
            ..CodegenOptions::new(dialect)
        };
        let outcome = codegen::run(&resources(), &options, &mut NonInteractive).unwrap();
        assert_eq!(outcome, CodegenOutcome::NoChanges, "{dialect:?} migrations are behind the resources");
    }
}

/// A directory holding only the first migration, for one dialect.
fn first_migration(dialect: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for entry in std::fs::read_dir(lagrange::migrations_dir()).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.contains("_chart_the_sol_system.") && name.contains(&format!(".{dialect}.")) {
            std::fs::copy(&path, dir.path().join(&name)).unwrap();
        }
    }
    dir
}

const SEED_V1: &str = "
INSERT INTO planets (id, name, surface_gravity, position, atmosphere, created_at, updated_at)
  VALUES ('00000000-0000-0000-0000-000000000001', 'Ceres', 0.28, '[0.5,2.7,0.2]', 'vacuum', 'now', 'now');
INSERT INTO ports (id, planet_id, code, name, kind, relay, created_at, updated_at)
  VALUES ('00000000-0000-0000-0000-000000000002', '00000000-0000-0000-0000-000000000001', 'PZZ', 'Piazzi Station', 'geostationary', '10.7.0.1', 'now', 'now');
INSERT INTO docks (id, port_id, code, clamp, max_mass_tonnes, version)
  VALUES ('00000000-0000-0000-0000-000000000003', '00000000-0000-0000-0000-000000000002', 'A1', 'standard', 8000, 1);
INSERT INTO ships (id, line, registry, name, class, dry_mass_tonnes, slot_capacity, version, status, created_at, updated_at)
  VALUES ('00000000-0000-0000-0000-000000000004', 'helios-freight', 'HF-001', 'Long Haul', 'freighter', 7500, 6, 1, 'docked', '2187-01-01T10:00:00+02:00', 'now');
INSERT INTO berth_reservations (id, line, port_id, dock_code, ship_id, starts_at, ends_at, status, created_at, updated_at)
  VALUES ('00000000-0000-0000-0000-000000000005', 'helios-freight', '00000000-0000-0000-0000-000000000002', 'A1', '00000000-0000-0000-0000-000000000004', '2187-02-12T00:00:00Z', '2187-02-13T00:00:00Z', 'active', 'now', 'now');
";

#[tokio::test]
async fn sqlite_history_renames_without_losing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let db = Sqlite::file(dir.path().join("fleet.db")).await.unwrap();
    let v1 = first_migration("sqlite");
    assert_eq!(db.migrate(v1.path()).await.unwrap().len(), 1);
    sqlx::raw_sql(SEED_V1).execute(db.pool().unwrap()).await.unwrap();

    assert_eq!(db.migrate(lagrange::migrations_dir()).await.unwrap().len(), 2);
    let code: String = sqlx::query_scalar("SELECT berth_code FROM berth_reservations")
        .fetch_one(db.pool().unwrap())
        .await
        .unwrap();
    assert_eq!(code, "A1");
    let berths: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM berths").fetch_one(db.pool().unwrap()).await.unwrap();
    assert_eq!(berths, 1);

    // Down migrations undo the whole history.
    while db.rollback(lagrange::migrations_dir()).await.unwrap().is_some() {}
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name NOT LIKE '\\_ash%' ESCAPE '\\' AND name NOT LIKE 'sqlite%'",
    )
    .fetch_one(db.pool().unwrap())
    .await
    .unwrap();
    assert_eq!(tables, 0);
}

#[tokio::test]
async fn postgres_history_renames_without_losing_rows() {
    let Ok(base) = std::env::var("DATABASE_URL") else {
        assert!(std::env::var("CI").is_err(), "CI must set DATABASE_URL");
        return;
    };
    let admin = Postgres::connect(&base).await.unwrap();
    let schema = format!("lagrange_history_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE SCHEMA \"{schema}\"")).execute(admin.pool().unwrap()).await.unwrap();
    let separator = if base.contains('?') { '&' } else { '?' };
    let db = Postgres::connect(&format!("{base}{separator}options=-c%20search_path%3D{schema}%2Cpublic"))
        .await
        .unwrap();

    let v1 = first_migration("postgres");
    assert_eq!(db.migrate(v1.path()).await.unwrap().len(), 1);
    sqlx::raw_sql(SEED_V1).execute(db.pool().unwrap()).await.unwrap();
    assert_eq!(db.migrate(lagrange::migrations_dir()).await.unwrap().len(), 2);
    // Timestamps are `timestamptz`, so an offset is kept as the same instant.
    let commissioned: bool =
        sqlx::query_scalar("SELECT created_at = '2187-01-01T08:00:00Z'::timestamptz FROM ships")
            .fetch_one(db.pool().unwrap())
            .await
            .unwrap();
    assert!(commissioned);

    let code: String = sqlx::query_scalar("SELECT berth_code FROM berth_reservations")
        .fetch_one(db.pool().unwrap())
        .await
        .unwrap();
    assert_eq!(code, "A1");
    // The rename carried the constraints along, including the exclusion constraint.
    let constraints: Vec<String> = sqlx::query_scalar(
        "SELECT conname::text FROM pg_constraint WHERE connamespace = current_schema()::regnamespace AND (conrelid = 'berths'::regclass OR conrelid = 'berth_reservations'::regclass) ORDER BY 1",
    )
    .fetch_all(db.pool().unwrap())
    .await
    .unwrap();
    for expected in ["berth_reservations_no_overlap", "ck_berths_positive_mass", "fk_berth_reservations_berth", "fk_berths_port"] {
        assert!(constraints.iter().any(|c| c == expected), "{expected} in {constraints:?}");
    }

    while db.rollback(lagrange::migrations_dir()).await.unwrap().is_some() {}
    let tables: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_tables WHERE schemaname = current_schema() AND tablename NOT LIKE '\\_ash%'",
    )
    .fetch_one(db.pool().unwrap())
    .await
    .unwrap();
    assert_eq!(tables, 0);
}
