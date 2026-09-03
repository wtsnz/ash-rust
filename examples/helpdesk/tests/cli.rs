use std::process::Command;

use uuid::Uuid;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_helpdesk"))
}

fn stdout(cmd: &mut Command) -> String {
    let output = cmd.output().expect("run helpdesk");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn open_assign_list_across_processes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("helpdesk.db");
    let db = db.to_str().unwrap();
    let customer = Uuid::new_v4();

    let alice_out = stdout(bin().args(["--db", db, "rep", "add", "Alice"]));
    let alice: Uuid = alice_out
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();

    let open_out = stdout(bin().args([
        "--db",
        db,
        "--as",
        &customer.to_string(),
        "ticket",
        "open",
        "Printer",
        "is",
        "jammed",
    ]));
    assert!(open_out.contains("Printer is jammed"));
    let ticket: Uuid = open_out.split_whitespace().nth(1).unwrap().parse().unwrap();

    let list_as_other = stdout(bin().args([
        "--db",
        db,
        "--as",
        &Uuid::new_v4().to_string(),
        "ticket",
        "list",
    ]));
    assert!(list_as_other.contains("(none)"));

    stdout(bin().args([
        "--db",
        db,
        "--as",
        &alice.to_string(),
        "ticket",
        "assign",
        &ticket.to_string(),
        &alice.to_string(),
    ]));

    let listed = stdout(bin().args([
        "--db",
        db,
        "--as",
        &alice.to_string(),
        "ticket",
        "list",
        "--with-rep",
    ]));
    assert!(listed.contains(&ticket.to_string()));
    assert!(listed.contains("assignee_name=Alice"));
}
