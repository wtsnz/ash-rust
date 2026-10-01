use ash_sql::MigrationExecutor;

use crate::support::{Db, Project, TestDb, on_every_backend};

fn write_migration(dir: &std::path::Path, version: &str, name: &str, up: &str, down: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{version}_{name}.up.sql")), up).unwrap();
    std::fs::write(dir.join(format!("{version}_{name}.down.sql")), down).unwrap();
}

async fn failed_migration_leaves_no_partial_changes(db: TestDb) {
    let project = Project::for_db(&db);
    write_migration(
        &project.migrations(),
        "20260101000000",
        "create_notes",
        "CREATE TABLE notes (id TEXT PRIMARY KEY);",
        "DROP TABLE notes;",
    );
    write_migration(
        &project.migrations(),
        "20260101000001",
        "broken",
        "CREATE TABLE half_done (id TEXT PRIMARY KEY);\nSELECT 1 FROM __ash_missing_table;",
        "DROP TABLE half_done;",
    );

    let result = db.migrate(&project.migrations()).await;

    assert!(result.is_err(), "the second migration must fail");
    assert_eq!(db.schema().await.table_names(), vec!["notes"]);
    assert_eq!(db.applied_versions().await, vec!["20260101000000"]);
}
on_every_backend!(failed_migration_leaves_no_partial_changes);

async fn concurrent_migrators_apply_each_migration_once(db: TestDb) {
    let project = Project::for_db(&db);
    write_migration(
        &project.migrations(),
        "20260101000000",
        "create_notes",
        "CREATE TABLE notes (id TEXT PRIMARY KEY);",
        "DROP TABLE notes;",
    );
    write_migration(
        &project.migrations(),
        "20260101000001",
        "add_body",
        "ALTER TABLE notes ADD COLUMN body TEXT;",
        "ALTER TABLE notes DROP COLUMN body;",
    );

    let a = db.reconnect().await;
    let b = db.reconnect().await;
    let c = db.reconnect().await;
    let d = db.reconnect().await;
    let dir = project.migrations();
    let (a, b, c, d) = tokio::join!(
        a.migrate(&dir),
        b.migrate(&dir),
        c.migrate(&dir),
        d.migrate(&dir),
    );
    let results = [a, b, c, d];

    for result in &results {
        assert!(result.is_ok(), "every instance should succeed: {result:?}");
    }
    let applied: usize = results.iter().map(|r| r.as_ref().unwrap().len()).sum();
    assert_eq!(
        applied, 2,
        "each migration is applied by exactly one instance"
    );
    assert_eq!(db.applied_versions().await.len(), 2);
    assert!(db.schema().await.has_column("notes", "body"));
}
on_every_backend!(concurrent_migrators_apply_each_migration_once);

async fn hand_written_migrations_run_and_roll_back(db: TestDb) {
    let project = Project::for_db(&db);
    write_migration(
        &project.migrations(),
        "20260101000000",
        "create_notes",
        "CREATE TABLE notes (id TEXT PRIMARY KEY, body TEXT NOT NULL);",
        "DROP TABLE notes;",
    );

    assert_eq!(
        db.migrate(&project.migrations()).await.unwrap(),
        vec!["20260101000000"]
    );
    assert!(db.migrate(&project.migrations()).await.unwrap().is_empty());
    assert_eq!(db.schema().await.table_names(), vec!["notes"]);

    assert_eq!(
        db.rollback(&project.migrations()).await.unwrap().as_deref(),
        Some("20260101000000")
    );
    assert!(db.schema().await.tables.is_empty());
    assert!(db.applied_versions().await.is_empty());
}
on_every_backend!(hand_written_migrations_run_and_roll_back);

async fn applied_versions(db: &TestDb) -> Vec<String> {
    match &db.db {
        Db::Sqlite(sqlite) => sqlite.applied_versions().await,
        Db::Postgres(pg) => pg.applied_versions().await,
    }
    .unwrap()
}

async fn apply(db: &TestDb, sql: &str, version: &str) -> ash_core::Result<bool> {
    match &db.db {
        Db::Sqlite(sqlite) => sqlite.apply_migration(sql, version, "step").await,
        Db::Postgres(pg) => pg.apply_migration(sql, version, "step").await,
    }
}

async fn revert(db: &TestDb, sql: &str, version: &str) -> ash_core::Result<bool> {
    match &db.db {
        Db::Sqlite(sqlite) => sqlite.revert_migration(sql, version).await,
        Db::Postgres(pg) => pg.revert_migration(sql, version).await,
    }
}

/// Migrators on separate hosts share no process or file lock, so the database alone must
/// keep each step to one of them.
async fn migrators_on_separate_hosts_take_turns_per_step(db: TestDb) {
    let slow = match db.db {
        Db::Sqlite(_) => "",
        Db::Postgres(_) => "SELECT pg_sleep(0.3);",
    };
    applied_versions(&db).await;
    db.exec("CREATE TABLE ledger (entry TEXT)").await.unwrap();
    let up = format!("{slow} INSERT INTO ledger VALUES ('up');");
    let down = format!("{slow} INSERT INTO ledger VALUES ('down');");

    let hosts = [db.reconnect().await, db.reconnect().await, db.reconnect().await];
    let (a, b, c) = tokio::join!(
        apply(&hosts[0], &up, "20260101000000"),
        apply(&hosts[1], &up, "20260101000000"),
        apply(&hosts[2], &up, "20260101000000"),
    );
    let ran: Vec<bool> = [a, b, c].into_iter().map(Result::unwrap).collect();
    assert_eq!(ran.iter().filter(|ran| **ran).count(), 1, "{ran:?}");
    assert_eq!(db.int("SELECT COUNT(*) FROM ledger WHERE entry = 'up'").await, 1);
    assert_eq!(applied_versions(&db).await, ["20260101000000"]);

    let (a, b, c) = tokio::join!(
        revert(&hosts[0], &down, "20260101000000"),
        revert(&hosts[1], &down, "20260101000000"),
        revert(&hosts[2], &down, "20260101000000"),
    );
    let ran: Vec<bool> = [a, b, c].into_iter().map(Result::unwrap).collect();
    assert_eq!(ran.iter().filter(|ran| **ran).count(), 1, "{ran:?}");
    assert_eq!(db.int("SELECT COUNT(*) FROM ledger WHERE entry = 'down'").await, 1);
    assert!(applied_versions(&db).await.is_empty());

    // Only the latest version can be reverted.
    for version in ["20260101000000", "20260101000001"] {
        assert!(apply(&db, "", version).await.unwrap());
    }
    assert!(!revert(&db, "", "20260101000000").await.unwrap());
    assert_eq!(applied_versions(&db).await.len(), 2);
}
on_every_backend!(migrators_on_separate_hosts_take_turns_per_step);

/// A failed step reports the script's own error and leaves nothing locked or recorded.
async fn failed_steps_report_their_error_and_release_the_lock(db: TestDb) {
    let project = Project::for_db(&db);
    write_migration(
        &project.migrations(),
        "20260101000000",
        "broken",
        "SELECT 1 FROM __ash_missing_table;",
        "",
    );
    let err = db.migrate(&project.migrations()).await.unwrap_err();
    assert!(err.to_string().contains("__ash_missing_table"), "unexpected error: {err}");
    assert!(applied_versions(&db).await.is_empty());

    let other_host = db.reconnect().await;
    let applied = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        apply(&other_host, "CREATE TABLE fixed (id TEXT);", "20260101000000"),
    )
    .await
    .expect("the failed step must not leave the lock held");
    assert!(applied.unwrap());
}
on_every_backend!(failed_steps_report_their_error_and_release_the_lock);
