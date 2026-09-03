use std::fs;
use uuid::Uuid;

#[tokio::test]
async fn test_cli_generate_dry_run_and_write() {
    let temp_dir = std::env::temp_dir().join(format!("cargo_ash_test_{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&temp_dir).unwrap();

    let generate_args = cargo_ash::GenerateArgs {
        name: "create_users".to_string(),
        dialect: "sqlite".to_string(),
        dir: temp_dir.clone(),
        snapshots_dir: temp_dir.clone(),
        from: None,
        to: None,
        dry_run: false,
    };

    cargo_ash::run_generate(generate_args).unwrap();

    let entries: Vec<_> = fs::read_dir(&temp_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();

    assert_eq!(entries.len(), 2);
    assert!(entries.iter().any(|f| f.ends_with("_create_users.up.sql")));
    assert!(entries.iter().any(|f| f.ends_with("_create_users.down.sql")));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_cli_migrate_status_rollback_lifecycle() {
    let temp_dir = std::env::temp_dir().join(format!("cargo_ash_lifecycle_{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&temp_dir).unwrap();

    let db_path = temp_dir.join("test.db");
    let db_url = format!("sqlite://{}", db_path.display());

    let v1 = "20260903000001";
    let up_file = temp_dir.join(format!("{v1}_create_todos.up.sql"));
    let down_file = temp_dir.join(format!("{v1}_create_todos.down.sql"));

    fs::write(&up_file, "CREATE TABLE todos (id TEXT PRIMARY KEY, title TEXT NOT NULL);").unwrap();
    fs::write(&down_file, "DROP TABLE todos;").unwrap();

    // 1. Check status before migrate (pending)
    cargo_ash::run_status(cargo_ash::StatusArgs {
        database_url: Some(db_url.clone()),
        dir: temp_dir.clone(),
    })
    .await
    .unwrap();

    // 2. Run migrate
    cargo_ash::run_migrate(cargo_ash::MigrateArgs {
        database_url: Some(db_url.clone()),
        dir: temp_dir.clone(),
    })
    .await
    .unwrap();

    // 3. Check status after migrate (applied)
    cargo_ash::run_status(cargo_ash::StatusArgs {
        database_url: Some(db_url.clone()),
        dir: temp_dir.clone(),
    })
    .await
    .unwrap();

    // 4. Dump schema
    let dump_dir = temp_dir.join("snapshots");
    cargo_ash::run_dump(cargo_ash::DumpArgs {
        database_url: Some(db_url.clone()),
        output: dump_dir.clone(),
    })
    .await
    .unwrap();

    let todo_snapshot_file = dump_dir.join("todos.json");
    assert!(todo_snapshot_file.exists());
    let snapshot_content = fs::read_to_string(todo_snapshot_file).unwrap();
    let snapshot: ash_sql::TableSnapshot = serde_json::from_str(&snapshot_content).unwrap();
    assert_eq!(snapshot.table, "todos");
    assert_eq!(snapshot.columns.len(), 2);

    // 5. Rollback
    cargo_ash::run_rollback(cargo_ash::RollbackArgs {
        database_url: Some(db_url.clone()),
        dir: temp_dir.clone(),
        to: None,
    })
    .await
    .unwrap();

    // Cleanup
    let _ = fs::remove_dir_all(&temp_dir);
}
