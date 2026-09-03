use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_kanban_cli_workflow() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("kanban_test.db");
    let db_str = db_path.to_str().unwrap();

    let exe = env!("CARGO_BIN_EXE_kanban");

    // 1. Register a user
    let out = Command::new(exe)
        .args(["--db", db_str, "user", "register", "Alice", "alice@corp.com"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Registered user: Alice"));

    // Extract user id from stdout
    let id_prefix = "[id: ";
    let id_start = stdout.find(id_prefix).unwrap() + id_prefix.len();
    let id_end = stdout[id_start..].find(']').unwrap() + id_start;
    let alice_id = &stdout[id_start..id_end];

    // 2. Create workspace as Alice
    let out = Command::new(exe)
        .args([
            "--db", db_str, "--as", alice_id, "workspace", "create", "Core Tech", "core-tech",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Created workspace 'Core Tech'"));

    let id_start = stdout.find(id_prefix).unwrap() + id_prefix.len();
    let id_end = stdout[id_start..].find(']').unwrap() + id_start;
    let ws_id = &stdout[id_start..id_end];

    // 3. Create board using template
    let out = Command::new(exe)
        .args([
            "--db", db_str, "--as", alice_id, "board", "create", ws_id, "Sprint 1", "--template",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Created template board 'Sprint 1'"));

    // 4. List boards
    let out = Command::new(exe)
        .args(["--db", db_str, "--as", alice_id, "board", "list"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("Sprint 1"));
    assert!(stdout.contains("lists: 3"));
    assert!(stdout.contains("cards: 1"));
}
