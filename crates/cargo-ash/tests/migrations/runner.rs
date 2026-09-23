use crate::support::{Project, TestDb, on_every_backend};

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
