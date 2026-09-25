use ash_core::{DataLayer, FieldMap, Resource, Value};
use cargo_ash::codegen::{
    CodegenError, CodegenOptions, CodegenOutcome, Mode, NonInteractive, RenameQuestion, Resolution,
};

use crate::fixtures::{self, ORG_ID, OTHER_ORG_ID, TICKET_ID};
use crate::support::{Column, Db, ForeignKey, Project, TestDb, on_every_backend};

async fn seed_ticket(db: &TestDb) {
    db.exec(&format!(
        "INSERT INTO orgs (id, name) VALUES ('{ORG_ID}', 'Acme')"
    ))
    .await
    .unwrap();
    db.exec(&format!(
        "INSERT INTO tickets (id, subject, status, notes, org_id) VALUES ('{TICKET_ID}', 'Printer on fire', 'open', 'call facilities', '{ORG_ID}')"
    ))
    .await
    .unwrap();
}

async fn initial_codegen_creates_every_table(db: TestDb) {
    let project = Project::for_db(&db);

    let migration = project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();

    let dialect = db.dialect().name();
    assert_eq!(
        project.migration_files(),
        vec![
            format!("{}_create_helpdesk.{dialect}.down.sql", migration.version),
            format!("{}_create_helpdesk.{dialect}.up.sql", migration.version),
        ]
    );
    assert_eq!(project.snapshot_files(), vec!["orgs.json", "tickets.json"]);

    let schema = db.schema().await;
    assert_eq!(schema.table_names(), vec!["orgs", "tickets"]);
    assert!(schema.column("tickets", "id").primary_key);
    assert_eq!(
        schema.column("tickets", "status"),
        &Column {
            ty: db.text_type().into(),
            nullable: false,
            default: Some("'open'".into()),
            primary_key: false
        }
    );
    assert_eq!(
        schema.column("tickets", "notes"),
        &Column {
            ty: db.text_type().into(),
            nullable: true,
            default: None,
            primary_key: false
        }
    );
    assert_eq!(schema.column("tickets", "org_id").ty, db.uuid_type());
    assert_eq!(
        schema
            .table("tickets")
            .unique_indexes
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        vec![vec!["subject".to_string()]]
    );
    assert_eq!(
        schema.table("tickets").foreign_keys,
        vec![ForeignKey {
            column: "org_id".into(),
            references_table: "orgs".into(),
            references_column: "id".into(),
            on_delete: "CASCADE".into(),
        }]
    );

    db.rollback(&project.migrations()).await.unwrap();
    assert!(db.schema().await.tables.is_empty());
}
on_every_backend!(initial_codegen_creates_every_table);

async fn codegen_without_changes_writes_nothing(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    let files_before = project.migration_files();

    let options = project.options(Mode::Write, Some("again"));
    let outcome = project
        .run(&options, &fixtures::helpdesk(), &mut NonInteractive)
        .unwrap();

    assert_eq!(outcome, CodegenOutcome::NoChanges);
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(codegen_without_changes_writes_nothing);

async fn check_mode_reports_pending_changes_without_writing(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    let files_before = project.migration_files();
    let snapshot_before = std::fs::read_to_string(
        project
            .snapshots()
            .join(db.dialect().name())
            .join("tickets.json"),
    )
    .unwrap();

    let check = project.options(Mode::Check, None);
    let unchanged = project
        .run(&check, &fixtures::helpdesk(), &mut NonInteractive)
        .unwrap();
    let changed = project
        .run(
            &check,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();

    assert_eq!(unchanged.exit_code(), 0);
    match &changed {
        CodegenOutcome::OutOfDate { up_sql } => assert!(up_sql.contains("priority"), "{up_sql}"),
        other => panic!("expected OutOfDate, got {other:?}"),
    }
    assert_eq!(changed.exit_code(), 1);
    assert_eq!(project.migration_files(), files_before);
    assert_eq!(
        std::fs::read_to_string(
            project
                .snapshots()
                .join(db.dialect().name())
                .join("tickets.json")
        )
        .unwrap(),
        snapshot_before
    );
}
on_every_backend!(check_mode_reports_pending_changes_without_writing);

async fn dry_run_returns_sql_without_writing(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    let files_before = project.migration_files();

    let options = project.options(Mode::DryRun, Some("add_priority"));
    let outcome = project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();

    match outcome {
        CodegenOutcome::WouldWrite { up_sql, down_sql } => {
            assert!(up_sql.contains("priority"), "{up_sql}");
            assert!(down_sql.contains("priority"), "{down_sql}");
        }
        other => panic!("expected WouldWrite, got {other:?}"),
    }
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(dry_run_returns_sql_without_writing);

async fn writing_changes_requires_a_name(db: TestDb) {
    let project = Project::for_db(&db);
    let options = project.options(Mode::Write, None);

    let result = project.run(&options, &fixtures::helpdesk(), &mut NonInteractive);

    assert!(
        matches!(result, Err(CodegenError::MissingName)),
        "{result:?}"
    );
    assert!(project.migration_files().is_empty());
    assert!(project.snapshot_files().is_empty());
}
on_every_backend!(writing_changes_requires_a_name);

async fn adding_attribute_with_default_backfills_existing_rows(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    project.generate(
        "add_priority",
        &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );
    assert_eq!(
        db.schema()
            .await
            .column("tickets", "priority")
            .default
            .as_deref(),
        Some("3")
    );

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
}
on_every_backend!(adding_attribute_with_default_backfills_existing_rows);

async fn runtime_default_is_not_written_to_the_schema(db: TestDb) {
    let project = Project::for_db(&db);

    project.generate(
        "create_helpdesk",
        &fixtures::helpdesk_with(&fixtures::with_runtime_default::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();

    let code = db.schema().await.column("tickets", "code").clone();
    assert_eq!(
        code,
        Column {
            ty: db.text_type().into(),
            nullable: false,
            default: None,
            primary_key: false
        }
    );
}
on_every_backend!(runtime_default_is_not_written_to_the_schema);

async fn removed_attribute_keeps_its_column_by_default(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let migration = project.generate(
        "remove_notes",
        &fixtures::helpdesk_with(&fixtures::without_notes::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();

    let schema = db.schema().await;
    assert!(schema.has_column("tickets", "notes"));
    assert_eq!(
        schema.column("tickets", "status").default.as_deref(),
        Some("'new'")
    );
    assert_eq!(
        db.text(&format!(
            "SELECT notes FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "call facilities"
    );
    assert!(
        migration.up_sql.contains("-- ALTER TABLE"),
        "the drop should be written but commented out:\n{}",
        migration.up_sql
    );

    let options = project.options(Mode::Check, None);
    let outcome = project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::without_notes::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    assert_eq!(outcome, CodegenOutcome::NoChanges);
}
on_every_backend!(removed_attribute_keeps_its_column_by_default);

async fn drop_columns_option_drops_removed_columns(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    let options = CodegenOptions {
        drop_columns: true,
        ..project.options(Mode::Write, Some("drop_notes"))
    };
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::without_notes::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    assert!(!db.schema().await.has_column("tickets", "notes"));
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
}
on_every_backend!(drop_columns_option_drops_removed_columns);

async fn confirmed_rename_preserves_data(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    let mut asked = Vec::new();
    let mut resolver = |question: &RenameQuestion| {
        asked.push(question.clone());
        Resolution::RenamedFrom("subject".into())
    };
    let options = project.options(Mode::Write, Some("rename_subject"));
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::renamed_subject::Ticket::DEF),
            &mut resolver,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(
        asked,
        vec![RenameQuestion {
            table: "tickets".into(),
            added: "title".into(),
            candidates: vec!["subject".into()]
        }]
    );
    assert_eq!(
        db.text(&format!(
            "SELECT title FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
    assert!(!db.schema().await.has_column("tickets", "subject"));

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
}
on_every_backend!(confirmed_rename_preserves_data);

async fn declined_rename_adds_a_new_column(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();

    let mut resolver = |_: &RenameQuestion| Resolution::NotRenamed;
    let options = project.options(Mode::Write, Some("add_title"));
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::renamed_subject::Ticket::DEF),
            &mut resolver,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    let schema = db.schema().await;
    assert!(schema.has_column("tickets", "title"));
    assert!(schema.has_column("tickets", "subject"));
}
on_every_backend!(declined_rename_adds_a_new_column);

async fn unresolved_rename_fails_and_writes_nothing(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    let files_before = project.migration_files();

    let options = project.options(Mode::Write, Some("rename_subject"));
    let result = project.run(
        &options,
        &fixtures::helpdesk_with(&fixtures::renamed_subject::Ticket::DEF),
        &mut NonInteractive,
    );

    match result {
        Err(CodegenError::AmbiguousRenames(questions)) => assert_eq!(
            questions,
            vec![RenameQuestion {
                table: "tickets".into(),
                added: "title".into(),
                candidates: vec!["subject".into()]
            }]
        ),
        other => panic!("expected AmbiguousRenames, got {other:?}"),
    }
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(unresolved_rename_fails_and_writes_nothing);

async fn changing_nullability_and_default_keeps_data(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    project.generate(
        "require_notes",
        &fixtures::helpdesk_with(&fixtures::required_notes::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();

    let schema = db.schema().await;
    assert_eq!(
        schema.column("tickets", "notes"),
        &Column {
            ty: db.text_type().into(),
            nullable: false,
            default: Some("'none'".into()),
            primary_key: false
        }
    );
    assert_eq!(schema.table("tickets"), &{
        let mut expected = before.table("tickets").clone();
        expected
            .columns
            .insert("notes".into(), schema.column("tickets", "notes").clone());
        expected
    });
    assert_eq!(
        db.text(&format!(
            "SELECT notes FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "call facilities"
    );

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    assert_eq!(
        db.text(&format!(
            "SELECT notes FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "call facilities"
    );
}
on_every_backend!(changing_nullability_and_default_keeps_data);

async fn altering_a_parent_table_keeps_child_rows(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    project.generate(
        "default_org_name",
        &[
            &fixtures::org_with_default_name::Org::DEF,
            &fixtures::base::Ticket::DEF,
        ],
    );
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(
        db.schema().await.column("orgs", "name").default.as_deref(),
        Some("'unnamed'")
    );
    assert_eq!(db.int("SELECT COUNT(*) FROM tickets").await, 1);
    assert_eq!(db.schema().await.table("tickets"), before.table("tickets"));

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    assert_eq!(db.int("SELECT COUNT(*) FROM tickets").await, 1);
}
on_every_backend!(altering_a_parent_table_keeps_child_rows);

async fn changing_a_column_type_converts_existing_values(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_counters", &[&fixtures::counter_text::Counter::DEF]);
    db.migrate(&project.migrations()).await.unwrap();
    db.exec(&format!(
        "INSERT INTO counters (id, value) VALUES ('{TICKET_ID}', '42')"
    ))
    .await
    .unwrap();
    let before = db.schema().await;

    project.generate(
        "counter_value_integer",
        &[&fixtures::counter_integer::Counter::DEF],
    );
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(
        db.schema().await.column("counters", "value").ty,
        db.integer_type()
    );
    assert_eq!(db.int("SELECT value FROM counters").await, 42);

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    assert_eq!(db.text("SELECT value FROM counters").await, "42");
}
on_every_backend!(changing_a_column_type_converts_existing_values);

async fn adding_an_identity_enforces_uniqueness(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate(
        "create_helpdesk",
        &fixtures::helpdesk_with(&fixtures::without_identity::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let duplicate = format!(
        "INSERT INTO tickets (id, subject, status, org_id) VALUES ('00000000-0000-0000-0000-000000000002', 'Printer on fire', 'open', '{ORG_ID}')"
    );

    project.generate("unique_subject", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();

    assert!(
        db.exec(&duplicate).await.is_err(),
        "duplicate subject should be rejected"
    );

    db.rollback(&project.migrations()).await.unwrap();
    db.exec(&duplicate)
        .await
        .expect("duplicate subject is allowed once the identity is rolled back");
}
on_every_backend!(adding_an_identity_enforces_uniqueness);

async fn adding_belongs_to_creates_an_enforced_foreign_key(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate(
        "create_helpdesk",
        &fixtures::helpdesk_with(&fixtures::unlinked::Ticket::DEF),
    );
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;
    let before = db.schema().await;

    project.generate("link_ticket_org", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();

    let orphan = format!(
        "INSERT INTO tickets (id, subject, status, org_id) VALUES ('00000000-0000-0000-0000-000000000003', 'Orphan', 'open', '{OTHER_ORG_ID}')"
    );
    assert!(
        db.exec(&orphan).await.is_err(),
        "ticket pointing at a missing org should be rejected"
    );
    db.exec(&format!("DELETE FROM orgs WHERE id = '{ORG_ID}'"))
        .await
        .unwrap();
    assert_eq!(
        db.int("SELECT COUNT(*) FROM tickets").await,
        0,
        "on_delete: cascade removes the org's tickets"
    );

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    db.exec(&orphan)
        .await
        .expect("orphans are allowed once the foreign key is rolled back");
}
on_every_backend!(adding_belongs_to_creates_an_enforced_foreign_key);

async fn tables_are_created_in_dependency_order(db: TestDb) {
    let project = Project::for_db(&db);

    project.generate(
        "create_helpdesk",
        &[&fixtures::base::Ticket::DEF, &fixtures::base::Org::DEF],
    );
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(db.schema().await.table("tickets").foreign_keys.len(), 1);
    db.rollback(&project.migrations()).await.unwrap();
    assert!(db.schema().await.tables.is_empty());
}
on_every_backend!(tables_are_created_in_dependency_order);

async fn removing_a_resource_drops_its_table_and_rollback_restores_it(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    let before = db.schema().await;

    project.generate("drop_tickets", &[&fixtures::base::Org::DEF]);
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(db.schema().await.table_names(), vec!["orgs"]);
    assert_eq!(project.snapshot_files(), vec!["orgs.json"]);

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
}
on_every_backend!(removing_a_resource_drops_its_table_and_rollback_restores_it);

async fn install_and_migrations_produce_the_same_schema(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    let migrated = db.schema().await;

    let installed_db = match db.dialect() {
        cargo_ash::codegen::Dialect::Sqlite => TestDb::sqlite().await,
        cargo_ash::codegen::Dialect::Postgres => TestDb::postgres().await.unwrap(),
    };
    installed_db.install(&fixtures::helpdesk()).await.unwrap();

    assert_eq!(installed_db.schema().await, migrated);
}
on_every_backend!(install_and_migrations_produce_the_same_schema);

async fn migrations_only_run_on_their_own_dialect(db: TestDb) {
    let other = match db.dialect() {
        cargo_ash::codegen::Dialect::Sqlite => cargo_ash::codegen::Dialect::Postgres,
        cargo_ash::codegen::Dialect::Postgres => cargo_ash::codegen::Dialect::Sqlite,
    };
    let project = Project::new(other);
    project.generate("create_helpdesk", &fixtures::helpdesk());

    let applied = db.migrate(&project.migrations()).await.unwrap();

    assert!(
        applied.is_empty(),
        "applied {applied:?} from another dialect"
    );
    assert!(db.schema().await.tables.is_empty());
}
on_every_backend!(migrations_only_run_on_their_own_dialect);

#[test]
fn cli_check_fails_when_resources_changed() {
    let project = Project::new(cargo_ash::codegen::Dialect::Sqlite);
    let dirs = [
        "--dialect".to_string(),
        "sqlite".to_string(),
        "--migrations-dir".to_string(),
        project.migrations().display().to_string(),
        "--snapshots-dir".to_string(),
        project.snapshots().display().to_string(),
    ];
    let cli = |domain: &'static ash_core::DomainDef, extra: &[&str]| {
        let args = ["ash", "codegen"]
            .iter()
            .map(|s| s.to_string())
            .chain(extra.iter().map(|s| s.to_string()))
            .chain(dirs.clone());
        cargo_ash::codegen::run_cli(&[domain], args)
    };

    let written = cli(&<fixtures::base::Helpdesk>::DEF, &["create_helpdesk"]).unwrap();
    let clean = cli(&<fixtures::base::Helpdesk>::DEF, &["--check"]).unwrap();
    let stale = cli(&<fixtures::with_priority::Helpdesk>::DEF, &["--check"]).unwrap();

    assert!(matches!(written, CodegenOutcome::Written(_)), "{written:?}");
    assert_eq!(clean.exit_code(), 0);
    assert_eq!(stale.exit_code(), 1);
}

#[tokio::test]
async fn cli_rename_flag_resolves_a_rename() {
    let db = TestDb::sqlite().await;
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let args = [
        "ash",
        "codegen",
        "rename_subject",
        "--rename",
        "tickets.subject=title",
        "--dialect",
        "sqlite",
        "--migrations-dir",
        &project.migrations().display().to_string(),
        "--snapshots-dir",
        &project.snapshots().display().to_string(),
    ]
    .map(|s| s.to_string());
    cargo_ash::codegen::run_cli(&[&<fixtures::renamed_subject::Helpdesk>::DEF], args).unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    assert_eq!(
        db.text(&format!(
            "SELECT title FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
}

#[test]
fn successive_migrations_get_increasing_versions() {
    let project = Project::new(cargo_ash::codegen::Dialect::Sqlite);

    let first = project.generate(
        "create_helpdesk",
        &fixtures::helpdesk_with(&fixtures::without_identity::Ticket::DEF),
    );
    let second = project.generate("unique_subject", &fixtures::helpdesk());
    let third = project.generate(
        "add_priority",
        &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
    );

    assert!(
        first.version < second.version && second.version < third.version,
        "{} {} {}",
        first.version,
        second.version,
        third.version
    );
}

async fn dev_codegen_applies_without_a_name(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let tickets_snap = project
        .snapshots()
        .join(db.dialect().name())
        .join("tickets.json");
    let committed_before = std::fs::read_to_string(&tickets_snap).unwrap();

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    let outcome = project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    assert!(matches!(outcome, CodegenOutcome::Written(_)), "{outcome:?}");

    let applied = db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );
    assert_eq!(
        std::fs::read_to_string(&tickets_snap).unwrap(),
        committed_before
    );

    let second = db.migrate(&project.migrations()).await.unwrap();
    assert!(second.is_empty(), "second migrate applied {second:?}");
}
on_every_backend!(dev_codegen_applies_without_a_name);

async fn dev_without_changes_writes_nothing(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    let files_before = project.migration_files();

    let outcome = project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    assert_eq!(outcome, CodegenOutcome::NoChanges);
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(dev_without_changes_writes_nothing);

async fn second_dev_migration_only_adds_the_new_column(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );

    let second = match project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority_and_category::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected second dev write, got {other:?}"),
    };
    assert!(
        second.up_sql.to_ascii_lowercase().contains("category"),
        "{}",
        second.up_sql
    );
    assert!(
        !second.up_sql.to_ascii_lowercase().contains("create table"),
        "{}",
        second.up_sql
    );

    let applied = db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(applied.len(), 1);
    assert_eq!(db.applied_versions().await.len(), 3);
    assert!(db.schema().await.has_column("tickets", "category"));
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
}
on_every_backend!(second_dev_migration_only_adds_the_new_column);

async fn named_codegen_refuses_to_drop_dev_files_before_rollback(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    let files_before = project.migration_files();

    let named = project.options(Mode::Write, Some("add_priority"));
    let err = project
        .run(
            &named,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap_err();
    assert!(
        matches!(err, CodegenError::RollbackDevFirst { .. }),
        "{err:?}"
    );
    assert_eq!(project.migration_files(), files_before);
    assert!(db.migrate(&project.migrations()).await.unwrap().is_empty());
    assert!(db.schema().await.has_column("tickets", "priority"));

    let reverted = project
        .run(&named, &fixtures::helpdesk(), &mut NonInteractive)
        .unwrap_err();
    assert!(
        matches!(reverted, CodegenError::RollbackDevFirst { .. }),
        "{reverted:?}"
    );
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(named_codegen_refuses_to_drop_dev_files_before_rollback);

async fn named_codegen_squashes_dev_migrations_and_keeps_rows(db: TestDb) {
    let project = Project::for_db(&db);
    let create = project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority_and_category::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    db.rollback(&project.migrations()).await.unwrap();
    db.rollback(&project.migrations()).await.unwrap();
    assert!(!db.schema().await.has_column("tickets", "priority"));
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );

    for name in project.migration_files() {
        if name.contains("_dev.") {
            std::fs::remove_file(project.migrations().join(name)).unwrap();
        }
    }

    let named = project.options(Mode::Write, Some("add_priority_and_category"));
    let squash = match project
        .run(
            &named,
            &fixtures::helpdesk_with(&fixtures::with_priority_and_category::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected squash write, got {other:?}"),
    };
    assert!(
        project
            .migration_files()
            .iter()
            .all(|name| !name.contains("_dev.")),
        "{:?}",
        project.migration_files()
    );
    assert!(
        !project
            .snapshots()
            .join(db.dialect().name())
            .join("dev")
            .exists()
    );

    let applied = db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(applied, vec![squash.version.clone()]);
    assert_eq!(
        db.applied_versions().await,
        vec![create.version.clone(), squash.version.clone()]
    );
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );
    assert!(db.schema().await.has_column("tickets", "category"));

    db.rollback(&project.migrations()).await.unwrap();
    assert!(!db.schema().await.has_column("tickets", "priority"));
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
}
on_every_backend!(named_codegen_squashes_dev_migrations_and_keeps_rows);

fn confirm_subject_became_title(question: &RenameQuestion) -> Resolution {
    if question.added == "title" && question.candidates.iter().any(|name| name == "subject") {
        Resolution::RenamedFrom("subject".into())
    } else {
        Resolution::NotRenamed
    }
}

fn dev_resources_after_type_change() -> Vec<&'static ash_core::ResourceDef> {
    vec![
        &fixtures::base::Org::DEF,
        &fixtures::dev_final_ticket::Ticket::DEF,
        &fixtures::dev_comment::Comment::DEF,
    ]
}

async fn dev_squash_keeps_a_rename_a_type_change_and_a_new_child_table(db: TestDb) {
    let project = Project::for_db(&db);
    let create = project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut dev = project.options(Mode::Write, None);
    dev.dev = true;
    project
        .run(
            &dev,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );

    let renamed = match project
        .run(
            &dev,
            &fixtures::helpdesk_with(&fixtures::dev_renamed_estimate::Ticket::DEF),
            &mut confirm_subject_became_title,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected the rename dev migration, got {other:?}"),
    };
    assert!(
        !renamed
            .up_sql
            .to_ascii_lowercase()
            .contains("create table \"tickets\""),
        "{}",
        renamed.up_sql
    );
    db.migrate(&project.migrations()).await.unwrap();
    db.exec(&format!(
        "UPDATE tickets SET estimate = '42' WHERE id = '{TICKET_ID}'"
    ))
    .await
    .unwrap();
    assert_eq!(
        db.text(&format!(
            "SELECT title FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
    assert!(!db.schema().await.has_column("tickets", "subject"));

    let typed = match project
        .run(
            &dev,
            &dev_resources_after_type_change(),
            &mut confirm_subject_became_title,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected the type-change dev migration, got {other:?}"),
    };
    assert!(
        typed.up_sql.to_ascii_lowercase().contains("comments"),
        "{}",
        typed.up_sql
    );
    db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(
        db.schema().await.column("tickets", "estimate").ty,
        db.integer_type()
    );
    assert_eq!(
        db.int(&format!(
            "SELECT estimate FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        42
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "open"
    );
    assert_eq!(
        db.schema()
            .await
            .column("tickets", "status")
            .default
            .as_deref(),
        Some("'new'")
    );
    let comment_id = "00000000-0000-0000-0000-0000000000c1";
    db.exec(&format!(
        "INSERT INTO comments (id, body, ticket_id) VALUES ('{comment_id}', 'seen in dev', '{TICKET_ID}')"
    ))
    .await
    .unwrap();
    assert_eq!(db.applied_versions().await.len(), 4);

    for _ in 0..3 {
        db.rollback(&project.migrations()).await.unwrap();
    }
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT notes FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "call facilities"
    );
    let rolled_back = db.schema().await;
    assert!(!rolled_back.has_column("tickets", "title"));
    assert!(!rolled_back.has_column("tickets", "priority"));
    assert!(!rolled_back.has_column("tickets", "estimate"));
    assert!(!rolled_back.tables.contains_key("comments"));
    assert_eq!(
        rolled_back.column("tickets", "status").default.as_deref(),
        Some("'open'")
    );

    for name in project.migration_files() {
        if name.contains("_dev.") {
            std::fs::remove_file(project.migrations().join(name)).unwrap();
        }
    }

    let named = project.options(Mode::Write, Some("reshape_tickets"));
    let refused = project.run(
        &named,
        &dev_resources_after_type_change(),
        &mut NonInteractive,
    );
    assert!(
        matches!(refused, Err(CodegenError::AmbiguousRenames(_))),
        "{refused:?}"
    );
    assert_eq!(project.migration_files().len(), 2);

    let squash = match project
        .run(
            &named,
            &dev_resources_after_type_change(),
            &mut confirm_subject_became_title,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected one squash migration, got {other:?}"),
    };
    assert!(
        project
            .migration_files()
            .iter()
            .all(|name| !name.contains("_dev.")),
        "{:?}",
        project.migration_files()
    );

    db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(
        db.applied_versions().await,
        vec![create.version.clone(), squash.version.clone()]
    );
    assert_eq!(
        db.text(&format!(
            "SELECT title FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT notes FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "call facilities"
    );
    assert_eq!(
        db.text(&format!(
            "SELECT status FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "open"
    );
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );
    assert_eq!(
        db.int("SELECT COUNT(*) FROM tickets WHERE estimate IS NULL")
            .await,
        1
    );
    let schema = db.schema().await;
    assert!(!schema.has_column("tickets", "subject"));
    assert_eq!(schema.column("tickets", "estimate").ty, db.integer_type());
    assert_eq!(
        schema.column("tickets", "status").default.as_deref(),
        Some("'new'")
    );
    assert_eq!(
        schema
            .table("tickets")
            .unique_indexes
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        vec![vec!["title".to_string()]]
    );
    assert_eq!(
        schema.table("comments").foreign_keys,
        vec![ForeignKey {
            column: "ticket_id".into(),
            references_table: "tickets".into(),
            references_column: "id".into(),
            on_delete: "CASCADE".into(),
        }]
    );
    db.exec(&format!(
        "INSERT INTO comments (id, body, ticket_id) VALUES ('{comment_id}', 'after squash', '{TICKET_ID}')"
    ))
    .await
    .unwrap();
    assert!(
        db.exec(&format!(
            "INSERT INTO comments (id, body, ticket_id) VALUES ('00000000-0000-0000-0000-0000000000c2', 'orphan', '{OTHER_ORG_ID}')"
        ))
        .await
        .is_err(),
        "a comment needs a real ticket"
    );
    db.exec(&format!("DELETE FROM tickets WHERE id = '{TICKET_ID}'"))
        .await
        .unwrap();
    assert_eq!(db.int("SELECT COUNT(*) FROM comments").await, 0);

    db.rollback(&project.migrations()).await.unwrap();
    let restored = db.schema().await;
    assert!(restored.has_column("tickets", "subject"));
    assert!(!restored.has_column("tickets", "title"));
    assert!(!restored.has_column("tickets", "priority"));
    assert!(!restored.tables.contains_key("comments"));
    assert_eq!(
        restored.column("tickets", "status").default.as_deref(),
        Some("'open'")
    );
}
on_every_backend!(dev_squash_keeps_a_rename_a_type_change_and_a_new_child_table);

async fn check_sees_dev_work_as_pending(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    let files_before = project.migration_files();

    let resources = fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF);
    let check = project.options(Mode::Check, None);
    let stale = project
        .run(&check, &resources, &mut NonInteractive)
        .unwrap();
    match &stale {
        CodegenOutcome::OutOfDate { up_sql } => assert!(up_sql.contains("priority"), "{up_sql}"),
        other => panic!("expected OutOfDate, got {other:?}"),
    }

    let mut check_dev = project.options(Mode::Check, None);
    check_dev.dev = true;
    let current = project
        .run(&check_dev, &resources, &mut NonInteractive)
        .unwrap();
    assert_eq!(current, CodegenOutcome::NoChanges);
    assert_eq!(project.migration_files(), files_before);
}
on_every_backend!(check_sees_dev_work_as_pending);

async fn named_codegen_recovers_after_dev_files_were_deleted(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_helpdesk", &fixtures::helpdesk());
    db.migrate(&project.migrations()).await.unwrap();
    seed_ticket(&db).await;

    let mut options = project.options(Mode::Write, None);
    options.dev = true;
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority_and_category::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    db.rollback(&project.migrations()).await.unwrap();
    db.rollback(&project.migrations()).await.unwrap();

    let dialect = db.dialect().name();
    for name in project.migration_files() {
        if name.contains("_dev.") {
            std::fs::remove_file(project.migrations().join(name)).unwrap();
        }
    }
    let dev_dir = project.snapshots().join(dialect).join("dev");
    if dev_dir.exists() {
        std::fs::remove_dir_all(&dev_dir).unwrap();
    }

    let named = project.options(Mode::Write, Some("add_priority_and_category"));
    let squash = match project
        .run(
            &named,
            &fixtures::helpdesk_with(&fixtures::with_priority_and_category::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected recovery squash, got {other:?}"),
    };

    db.migrate(&project.migrations()).await.unwrap();
    assert_eq!(
        db.int(&format!(
            "SELECT priority FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        3
    );
    assert!(db.schema().await.has_column("tickets", "category"));
    assert_eq!(
        db.text(&format!(
            "SELECT subject FROM tickets WHERE id = '{TICKET_ID}'"
        ))
        .await,
        "Printer on fire"
    );
    let _ = squash;
}
on_every_backend!(named_codegen_recovers_after_dev_files_were_deleted);

const OPENED_AT: &str = "2024-01-02T03:04:05Z";
const INVOICE_ID: &str = "00000000-0000-0000-0000-0000000000d1";
const WRITTEN_INVOICE_ID: &str = "00000000-0000-0000-0000-0000000000d2";

fn same_decimal(got: &str, expected: &str) -> bool {
    fn canonical(raw: &str) -> Option<(bool, String, String)> {
        let negative = raw.starts_with('-');
        let raw = raw.strip_prefix(['+', '-']).unwrap_or(raw);
        let (whole, fraction) = raw.split_once('.').unwrap_or((raw, ""));
        if whole.is_empty() && fraction.is_empty() {
            return None;
        }
        let whole = whole.trim_start_matches('0');
        let whole = if whole.is_empty() { "0" } else { whole };
        let fraction = fraction.trim_end_matches('0');
        let negative = negative && !(whole == "0" && fraction.is_empty());
        Some((negative, whole.to_string(), fraction.to_string()))
    }
    canonical(got) == canonical(expected)
}

async fn opened_at_text(db: &TestDb, id: &str) -> String {
    match db.dialect().name() {
        "postgres" => {
            db.text(&format!(
                "SELECT to_char(opened_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') FROM invoices WHERE id = '{id}'"
            ))
            .await
        }
        _ => {
            db.text(&format!(
                "SELECT opened_at FROM invoices WHERE id = '{id}'"
            ))
            .await
        }
    }
}

async fn amount_text(db: &TestDb, id: &str) -> String {
    match db.dialect().name() {
        "postgres" => {
            db.text(&format!(
                "SELECT amount::text FROM invoices WHERE id = '{id}'"
            ))
            .await
        }
        _ => {
            db.text(&format!(
                "SELECT CAST(amount AS TEXT) FROM invoices WHERE id = '{id}'"
            ))
            .await
        }
    }
}

async fn invoice_rows(db: &TestDb) -> Vec<FieldMap> {
    let query = ash_core::CompiledQuery::default();
    match &db.db {
        crate::support::Db::Sqlite(sqlite) => sqlite
            .run_query(&fixtures::invoice::Invoice::DEF, &query)
            .await
            .unwrap(),
        crate::support::Db::Postgres(pg) => pg
            .run_query(&fixtures::invoice::Invoice::DEF, &query)
            .await
            .unwrap(),
    }
}

fn assert_invoice_values(row: &FieldMap, opened_at: &str, amount: &str) {
    match row.get("opened_at") {
        Some(Value::String(got)) => {
            assert_eq!(
                ash_core::UtcDateTime::parse(got).unwrap().as_str(),
                opened_at
            );
        }
        other => panic!("opened_at decoded as {other:?}"),
    }
    match row.get("amount") {
        Some(Value::String(got)) => {
            assert!(same_decimal(got, amount), "amount {got} is not {amount}");
            ash_core::Decimal::parse(got).unwrap();
        }
        other => panic!("amount decoded as {other:?}"),
    }
}

async fn codegen_creates_timestamptz_and_numeric_columns(db: TestDb) {
    let project = Project::for_db(&db);
    let migration = project.generate("create_invoices", &[&fixtures::invoice::Invoice::DEF]);
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_create_invoices.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    assert!(up.contains("\"opened_at\""));
    assert!(up.contains("\"amount\" NUMERIC") || up.contains("\"amount\" numeric"));
    if dialect == "postgres" {
        assert!(up.contains("TIMESTAMPTZ"));
    } else {
        assert!(up.contains("\"opened_at\" TEXT"));
    }
    assert!(up.contains("DEFAULT '12.50'"));

    db.migrate(&project.migrations()).await.unwrap();
    let schema = db.schema().await;
    assert_eq!(
        schema.column("invoices", "opened_at"),
        &Column {
            ty: db.datetime_type().into(),
            nullable: false,
            default: None,
            primary_key: false,
        }
    );
    assert_eq!(
        schema.column("invoices", "amount"),
        &Column {
            ty: db.decimal_type().into(),
            nullable: false,
            default: Some(
                if dialect == "postgres" {
                    "12.50"
                } else {
                    "'12.50'"
                }
                .into(),
            ),
            primary_key: false,
        }
    );

    db.exec(&format!(
        "INSERT INTO invoices (id, opened_at) VALUES ('{INVOICE_ID}', '{OPENED_AT}')"
    ))
    .await
    .unwrap();
    assert_eq!(opened_at_text(&db, INVOICE_ID).await, OPENED_AT);
    let defaulted = amount_text(&db, INVOICE_ID).await;
    assert!(
        same_decimal(&defaulted, "12.50"),
        "default amount was {defaulted}"
    );
    if dialect == "postgres" {
        assert_eq!(defaulted, "12.50");
    }

    let written_id = uuid::Uuid::parse_str(WRITTEN_INVOICE_ID).unwrap();
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(written_id));
    fields.insert("opened_at".into(), Value::String(OPENED_AT.into()));
    fields.insert("amount".into(), Value::String("7.25".into()));
    match &db.db {
        crate::support::Db::Sqlite(sqlite) => {
            sqlite
                .create(&fixtures::invoice::Invoice::DEF, written_id, fields)
                .await
                .unwrap();
        }
        crate::support::Db::Postgres(pg) => {
            pg.create(&fixtures::invoice::Invoice::DEF, written_id, fields)
                .await
                .unwrap();
        }
    }
    assert_eq!(opened_at_text(&db, WRITTEN_INVOICE_ID).await, OPENED_AT);
    let written = amount_text(&db, WRITTEN_INVOICE_ID).await;
    assert!(
        same_decimal(&written, "7.25"),
        "written amount was {written}"
    );

    let rows = invoice_rows(&db).await;
    assert_eq!(rows.len(), 2);
    let seeded = rows
        .iter()
        .find(|row| row.get("id") == Some(&Value::Uuid(uuid::Uuid::parse_str(INVOICE_ID).unwrap())))
        .unwrap();
    let written_row = rows
        .iter()
        .find(|row| row.get("id") == Some(&Value::Uuid(written_id)))
        .unwrap();
    assert_invoice_values(seeded, OPENED_AT, "12.50");
    assert_invoice_values(written_row, OPENED_AT, "7.25");

    db.rollback(&project.migrations()).await.unwrap();
    assert!(db.schema().await.tables.is_empty());
}
on_every_backend!(codegen_creates_timestamptz_and_numeric_columns);

async fn codegen_converts_text_values_into_datetime_and_decimal(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate("create_invoices", &[&fixtures::invoice_text::Invoice::DEF]);
    db.migrate(&project.migrations()).await.unwrap();
    db.exec(&format!(
        "INSERT INTO invoices (id, opened_at, amount) VALUES ('{INVOICE_ID}', '{OPENED_AT}', '12.50')"
    ))
    .await
    .unwrap();
    let before = db.schema().await;

    project.generate("invoice_typed_columns", &[&fixtures::invoice::Invoice::DEF]);
    db.migrate(&project.migrations()).await.unwrap();

    let schema = db.schema().await;
    assert_eq!(
        schema.column("invoices", "opened_at").ty,
        db.datetime_type()
    );
    assert_eq!(schema.column("invoices", "amount").ty, db.decimal_type());
    assert_eq!(opened_at_text(&db, INVOICE_ID).await, OPENED_AT);
    let amount = amount_text(&db, INVOICE_ID).await;
    assert!(
        same_decimal(&amount, "12.50"),
        "converted amount was {amount}"
    );
    if db.dialect().name() == "postgres" {
        assert_eq!(amount, "12.50");
    }
    let rows = invoice_rows(&db).await;
    assert_eq!(rows.len(), 1);
    assert_invoice_values(&rows[0], OPENED_AT, "12.50");

    db.rollback(&project.migrations()).await.unwrap();
    assert_eq!(db.schema().await, before);
    let restored_opened = db
        .text(&format!(
            "SELECT opened_at FROM invoices WHERE id = '{INVOICE_ID}'"
        ))
        .await;
    if db.dialect().name() == "postgres" {
        assert_eq!(
            db.int(&format!(
                "SELECT CASE WHEN opened_at::timestamptz = '2024-01-02T03:04:05Z'::timestamptz THEN 1::bigint ELSE 0::bigint END FROM invoices WHERE id = '{INVOICE_ID}'"
            ))
            .await,
            1,
            "restored opened_at was {restored_opened}"
        );
    } else {
        assert_eq!(restored_opened, OPENED_AT);
    }
    let restored_amount = db
        .text(&format!(
            "SELECT amount FROM invoices WHERE id = '{INVOICE_ID}'"
        ))
        .await;
    assert!(
        same_decimal(&restored_amount, "12.50"),
        "restored amount was {restored_amount}"
    );
}
on_every_backend!(codegen_converts_text_values_into_datetime_and_decimal);

async fn codegen_creates_a_non_unique_index(db: TestDb) {
    let project = Project::for_db(&db);
    let migration = project.generate(
        "create_labeled_notes",
        &[&fixtures::labeled_notes::LabeledNote::DEF],
    );
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_create_labeled_notes.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    assert!(
        up.contains("CREATE INDEX IF NOT EXISTS \"idx_labeled_notes_by_status\""),
        "up SQL missing non-unique index:\n{up}"
    );
    assert!(
        !up.contains("CREATE UNIQUE INDEX IF NOT EXISTS \"idx_labeled_notes_by_status\""),
        "by_status must not be unique:\n{up}"
    );
    assert!(
        up.contains("CREATE UNIQUE INDEX IF NOT EXISTS \"idx_labeled_notes_unique_title\""),
        "identity unique index missing:\n{up}"
    );

    db.migrate(&project.migrations()).await.unwrap();
    let schema = db.schema().await;
    assert_eq!(
        schema
            .table("labeled_notes")
            .indexes
            .get("idx_labeled_notes_by_status"),
        Some(&vec!["status".to_string()])
    );
    assert!(
        !schema
            .table("labeled_notes")
            .unique_indexes
            .contains_key("idx_labeled_notes_by_status")
    );
    assert_eq!(
        schema
            .table("labeled_notes")
            .unique_indexes
            .get("idx_labeled_notes_unique_title"),
        Some(&vec!["title".to_string()])
    );

    let note_a = "00000000-0000-0000-0000-0000000000a1";
    let note_b = "00000000-0000-0000-0000-0000000000a2";
    db.exec(&format!(
        "INSERT INTO labeled_notes (id, title, status) VALUES ('{note_a}', 'alpha', 'open')"
    ))
    .await
    .unwrap();
    db.exec(&format!(
        "INSERT INTO labeled_notes (id, title, status) VALUES ('{note_b}', 'beta', 'open')"
    ))
    .await
    .unwrap();
    let dup_identity = db
        .exec(
            "INSERT INTO labeled_notes (id, title, status) VALUES ('00000000-0000-0000-0000-0000000000a3', 'alpha', 'closed')",
        )
        .await;
    assert!(dup_identity.is_err(), "duplicate identity title must fail");

    db.rollback(&project.migrations()).await.unwrap();
    let after = db.schema().await;
    assert!(after.tables.is_empty() || !after.tables.contains_key("labeled_notes"));
}
on_every_backend!(codegen_creates_a_non_unique_index);

async fn codegen_adds_a_check_and_rejects_a_bad_row(db: TestDb) {
    {
        let create_project = Project::for_db(&db);
        let migration = create_project.generate(
            "create_bounded_notes_checked",
            &[&fixtures::bounded_notes::BoundedNote::DEF],
        );
        let dialect = db.dialect().name();
        let up = std::fs::read_to_string(create_project.migrations().join(format!(
            "{}_create_bounded_notes_checked.{dialect}.up.sql",
            migration.version
        )))
        .unwrap();
        assert!(
            up.contains("CONSTRAINT \"ck_bounded_notes_titled\" CHECK (title <> '')"),
            "create-table SQL missing check constraint:\n{up}"
        );
    }

    let project = Project::for_db(&db);
    project.generate(
        "create_bounded_notes",
        &[&fixtures::bounded_notes_plain::BoundedNote::DEF],
    );
    db.migrate(&project.migrations()).await.unwrap();

    let ok_id = "00000000-0000-0000-0000-0000000000b1";
    db.exec(&format!(
        "INSERT INTO bounded_notes (id, title) VALUES ('{ok_id}', 'ok')"
    ))
    .await
    .unwrap();

    let migration = project.generate(
        "add_title_check",
        &[&fixtures::bounded_notes::BoundedNote::DEF],
    );
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_add_title_check.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    if dialect == "postgres" {
        assert!(
            up.contains("ALTER TABLE")
                && up.contains("ADD CONSTRAINT")
                && up.contains("CHECK (title <> '')"),
            "postgres up should ALTER TABLE ADD CONSTRAINT CHECK:\n{up}"
        );
    } else {
        assert!(
            up.contains("CONSTRAINT \"ck_bounded_notes_titled\" CHECK (title <> '')")
                || up.contains("CHECK (title <> '')"),
            "sqlite rebuild up should recreate with CHECK:\n{up}"
        );
    }

    db.migrate(&project.migrations()).await.unwrap();
    let schema = db.schema().await;
    let check_expr = schema
        .table("bounded_notes")
        .checks
        .get("ck_bounded_notes_titled")
        .unwrap_or_else(|| panic!("missing check: {:?}", schema.table("bounded_notes").checks));
    assert!(
        check_expr.contains("title") && check_expr.contains("<>"),
        "unexpected check expression: {check_expr}"
    );

    let bad = db
        .exec(
            "INSERT INTO bounded_notes (id, title) VALUES ('00000000-0000-0000-0000-0000000000b2', '')",
        )
        .await;
    assert!(bad.is_err(), "empty title must fail under CHECK");

    assert_eq!(
        db.int(&format!(
            "SELECT COUNT(*) FROM bounded_notes WHERE id = '{ok_id}'"
        ))
        .await,
        1
    );

    db.rollback(&project.migrations()).await.unwrap();
    let after = db.schema().await;
    assert!(
        !after
            .table("bounded_notes")
            .checks
            .contains_key("ck_bounded_notes_titled"),
        "check should be gone after rollback: {:?}",
        after.table("bounded_notes").checks
    );

    db.exec(
        "INSERT INTO bounded_notes (id, title) VALUES ('00000000-0000-0000-0000-0000000000b2', '')",
    )
    .await
    .unwrap();
}
on_every_backend!(codegen_adds_a_check_and_rejects_a_bad_row);

async fn codegen_rejects_duplicate_check_name_without_writing(db: TestDb) {
    let project = Project::for_db(&db);
    let before = project.migration_files();
    let options = project.options(Mode::Write, Some("dup_check"));
    let result = project.run(
        &options,
        &[&fixtures::duplicate_check_notes::DuplicateCheckNote::DEF],
        &mut NonInteractive,
    );
    match result {
        Err(CodegenError::DuplicateCheckName { table, name }) => {
            assert_eq!(table, "duplicate_check_notes");
            assert_eq!(name, "titled");
        }
        other => panic!("expected DuplicateCheckName, got {other:?}"),
    }
    assert_eq!(
        project.migration_files(),
        before,
        "DuplicateCheckName must not write SQL files"
    );
    assert!(
        project.snapshot_files().is_empty(),
        "DuplicateCheckName must not write snapshot files"
    );
}
on_every_backend!(codegen_rejects_duplicate_check_name_without_writing);

async fn migrate_schemas_isolates_two_postgres_schemas(db: TestDb) {
    let project = Project::for_db(&db);
    project.generate(
        "create_bounded_notes",
        &[&fixtures::bounded_notes_plain::BoundedNote::DEF],
    );

    match &db.db {
        Db::Sqlite(_) => {
            eprintln!("SQLite did not call migrate_schemas");
            db.migrate(&project.migrations()).await.unwrap();
            let schema = db.schema().await;
            assert!(
                schema.tables.contains_key("bounded_notes"),
                "default migrate must still create the table on SQLite"
            );
        }
        Db::Postgres(pg) => {
            let caller_schema: String = sqlx::query_scalar("SELECT current_schema()")
                .fetch_one(pg.pool().unwrap())
                .await
                .unwrap();
            let schema_a = format!("tenant_a_{}", uuid::Uuid::new_v4().simple());
            let schema_b = format!("tenant_b_{}", uuid::Uuid::new_v4().simple());

            pg.migrate_schemas(&[&schema_a, &schema_b], project.migrations())
                .await
                .unwrap();

            let table_count = |schema: String| {
                let db = &db;
                async move {
                    db.int(&format!(
                        "SELECT COUNT(*) FROM pg_class c \
                         JOIN pg_namespace n ON n.oid = c.relnamespace \
                         WHERE n.nspname = '{schema}' AND c.relname = 'bounded_notes' AND c.relkind = 'r'"
                    ))
                    .await
                }
            };
            assert_eq!(
                table_count(schema_a.clone()).await,
                1,
                "schema A must have the table"
            );
            assert_eq!(
                table_count(schema_b.clone()).await,
                1,
                "schema B must have the table"
            );

            let row_id = "00000000-0000-0000-0000-0000000000a1";
            db.exec(&format!(
                "INSERT INTO \"{schema_a}\".bounded_notes (id, title) VALUES ('{row_id}', 'in A')"
            ))
            .await
            .unwrap();
            assert_eq!(
                db.int(&format!(
                    "SELECT COUNT(*) FROM \"{schema_b}\".bounded_notes WHERE id = '{row_id}'"
                ))
                .await,
                0,
                "schema B must not see a row inserted in schema A"
            );

            pg.migrate_schemas(&[&schema_a, &schema_b], project.migrations())
                .await
                .expect("second migrate_schemas call must be idempotent");

            let bad = "bad\"name";
            let err = pg
                .migrate_schemas(&[bad], project.migrations())
                .await
                .expect_err("schema names with quotes must be rejected");
            assert!(
                err.to_string().contains("invalid Postgres schema name"),
                "unexpected error: {err}"
            );
            assert_eq!(
                db.int("SELECT COUNT(*) FROM pg_namespace WHERE nspname = 'bad\"name'")
                    .await,
                0,
                "rejected name must not create a schema"
            );

            db.exec(&format!("SET search_path TO \"{caller_schema}\""))
                .await
                .unwrap();

            let base = std::env::var("DATABASE_URL").expect("DATABASE_URL");
            let separator = if base.contains('?') { '&' } else { '?' };
            let tenant_a_url = format!("{base}{separator}options=-c%20search_path%3D{schema_a}");
            let tenant_a = ash_postgres::Postgres::connect(&tenant_a_url)
                .await
                .unwrap();
            tenant_a.rollback(project.migrations()).await.unwrap();
            assert_eq!(
                table_count(schema_a.clone()).await,
                0,
                "rollback in schema A must drop its table"
            );
            assert_eq!(
                table_count(schema_b.clone()).await,
                1,
                "rollback in schema A must not drop schema B's table"
            );

            db.exec(&format!("SET search_path TO \"{caller_schema}\""))
                .await
                .unwrap();
            pg.migrate_schemas(&[&schema_a, &schema_b], project.migrations())
                .await
                .unwrap();

            let after_schema: String = sqlx::query_scalar("SELECT current_schema()")
                .fetch_one(pg.pool().unwrap())
                .await
                .unwrap();
            assert_eq!(
                after_schema, caller_schema,
                "migrate_schemas must restore the caller's search_path"
            );

            db.exec("CREATE TABLE post_migrate_probe (id int)")
                .await
                .unwrap();
            let probe_schema: String = sqlx::query_scalar(
                "SELECT n.nspname FROM pg_class c \
                 JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE c.relname = 'post_migrate_probe' AND c.relkind = 'r' \
                   AND n.nspname = current_schema()",
            )
            .fetch_one(pg.pool().unwrap())
            .await
            .unwrap();
            assert_eq!(
                probe_schema, caller_schema,
                "tables created after migrate_schemas must land in the test schema"
            );
            assert_ne!(probe_schema, schema_a);
            assert_ne!(probe_schema, schema_b);
        }
    }
}
on_every_backend!(migrate_schemas_isolates_two_postgres_schemas);

async fn setup_then_reset_recreates_an_empty_migrated_database(db: TestDb) {
    let project = Project::for_db(&db);
    let migration = project.generate(
        "create_bounded_notes",
        &[&fixtures::bounded_notes_plain::BoundedNote::DEF],
    );

    cargo_ash::run_setup(cargo_ash::MigrateArgs {
        database_url: Some(db.url.clone()),
        dir: project.migrations(),
    })
    .await
    .unwrap();

    let after_setup = db.applied_versions().await;
    assert_eq!(
        after_setup,
        vec![migration.version.clone()],
        "setup must record the latest version"
    );
    assert!(
        db.schema().await.tables.contains_key("bounded_notes"),
        "setup must create the resource table"
    );

    cargo_ash::run_setup(cargo_ash::MigrateArgs {
        database_url: Some(db.url.clone()),
        dir: project.migrations(),
    })
    .await
    .unwrap();
    assert_eq!(
        db.applied_versions().await,
        after_setup,
        "a second setup must not add a tracking row"
    );

    let row_id = "00000000-0000-0000-0000-0000000000b1";
    db.exec(&format!(
        "INSERT INTO bounded_notes (id, title) VALUES ('{row_id}', 'seed')"
    ))
    .await
    .unwrap();
    assert_eq!(
        db.int(&format!(
            "SELECT COUNT(*) FROM bounded_notes WHERE id = '{row_id}'"
        ))
        .await,
        1
    );

    cargo_ash::run_reset(cargo_ash::MigrateArgs {
        database_url: Some(db.url.clone()),
        dir: project.migrations(),
    })
    .await
    .unwrap();

    assert_eq!(
        db.int(&format!(
            "SELECT COUNT(*) FROM bounded_notes WHERE id = '{row_id}'"
        ))
        .await,
        0,
        "reset must drop the seeded row"
    );
    assert!(
        db.schema().await.tables.contains_key("bounded_notes"),
        "reset must leave the resource table in place after re-setup"
    );
    assert_eq!(
        db.applied_versions().await,
        vec![migration.version.clone()],
        "reset must leave the tracking table at the latest version"
    );
}
on_every_backend!(setup_then_reset_recreates_an_empty_migrated_database);

async fn squash_history_refuses_until_rollback_then_writes_one_migration(db: TestDb) {
    let project = Project::for_db(&db);
    let resources = fixtures::helpdesk();
    project.generate("create_helpdesk", &resources);
    db.migrate(&project.migrations()).await.unwrap();

    let options = project.options(Mode::Write, Some("add_priority"));
    project
        .run(
            &options,
            &fixtures::helpdesk_with(&fixtures::with_priority::Ticket::DEF),
            &mut NonInteractive,
        )
        .unwrap();
    db.migrate(&project.migrations()).await.unwrap();

    let files_before = project.migration_files();
    assert!(
        files_before.len() >= 4,
        "need a multi-file history before squash, got {files_before:?}"
    );

    let mut squash = project.options(Mode::Write, Some("create_helpdesk"));
    squash.squash_history = true;
    squash.applied_versions = Some(db.applied_versions().await);
    let refused = project
        .run(&squash, &resources, &mut NonInteractive)
        .unwrap_err();
    assert!(
        matches!(refused, CodegenError::RollbackBeforeSquash),
        "{refused:?}"
    );
    assert_eq!(
        project.migration_files(),
        files_before,
        "tracking rows must leave the migration files untouched"
    );
    assert!(
        refused
            .to_string()
            .contains("roll back until `_ash_schema_migrations` has no rows")
    );
    assert!(!refused.to_string().contains("reset"));

    let hand_written = project.migrations().join("notes.sql");
    std::fs::write(&hand_written, "SELECT 1;\n").unwrap();
    squash.applied_versions = Some(Vec::new());
    let hand = project
        .run(&squash, &resources, &mut NonInteractive)
        .unwrap_err();
    assert!(
        matches!(hand, CodegenError::HandWrittenMigration { .. }),
        "{hand:?}"
    );
    assert_eq!(
        project.migration_files(),
        {
            let mut expected = files_before.clone();
            expected.push("notes.sql".into());
            expected.sort();
            expected
        },
        "a hand-written file must leave prior migrations in place"
    );
    std::fs::remove_file(&hand_written).unwrap();

    while db.rollback(&project.migrations()).await.unwrap().is_some() {}
    assert!(
        db.applied_versions().await.is_empty(),
        "full rollback must clear `_ash_schema_migrations`"
    );

    squash.applied_versions = Some(db.applied_versions().await);
    let written = match project
        .run(&squash, &resources, &mut NonInteractive)
        .unwrap()
    {
        CodegenOutcome::Written(migration) => migration,
        other => panic!("expected squash write, got {other:?}"),
    };

    let dialect = db.dialect().name();
    assert_eq!(
        project.migration_files(),
        vec![
            format!("{}_create_helpdesk.{dialect}.down.sql", written.version),
            format!("{}_create_helpdesk.{dialect}.up.sql", written.version),
        ]
    );

    cargo_ash::run_setup(cargo_ash::MigrateArgs {
        database_url: Some(db.url.clone()),
        dir: project.migrations(),
    })
    .await
    .unwrap();

    let schema = db.schema().await;
    assert_eq!(schema.table_names(), vec!["orgs", "tickets"]);
    assert_eq!(
        db.applied_versions().await,
        vec![written.version.clone()],
        "setup on the empty database must apply the squashed migration"
    );
}
on_every_backend!(squash_history_refuses_until_rollback_then_writes_one_migration);

async fn codegen_creates_partial_indexes(db: TestDb) {
    let project = Project::for_db(&db);
    let migration = project.generate(
        "create_live_accounts",
        &[&fixtures::live_accounts::LiveAccount::DEF],
    );
    let dialect = db.dialect().name();
    let up = std::fs::read_to_string(project.migrations().join(format!(
        "{}_create_live_accounts.{dialect}.up.sql",
        migration.version
    )))
    .unwrap();
    assert!(
        up.contains(
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_live_accounts_live_email\" ON \"live_accounts\" (\"email\") WHERE deleted_at IS NULL;"
        ),
        "partial unique index missing:\n{up}"
    );
    assert!(
        up.contains(
            "CREATE INDEX IF NOT EXISTS \"idx_live_accounts_active_email\" ON \"live_accounts\" (\"email\") WHERE deleted_at IS NULL;"
        ),
        "partial index missing:\n{up}"
    );

    db.migrate(&project.migrations()).await.unwrap();
    db.exec(
        "INSERT INTO live_accounts (id, email, deleted_at) VALUES ('00000000-0000-0000-0000-0000000000a1', 'ada@example.com', NULL)",
    )
    .await
    .unwrap();
    let duplicate_live = db
        .exec(
            "INSERT INTO live_accounts (id, email, deleted_at) VALUES ('00000000-0000-0000-0000-0000000000a2', 'ada@example.com', NULL)",
        )
        .await;
    assert!(duplicate_live.is_err(), "live emails must stay unique");
    db.exec(
        "INSERT INTO live_accounts (id, email, deleted_at) VALUES ('00000000-0000-0000-0000-0000000000a3', 'ada@example.com', '2024-01-01')",
    )
    .await
    .unwrap();
    db.exec(
        "INSERT INTO live_accounts (id, email, deleted_at) VALUES ('00000000-0000-0000-0000-0000000000a4', 'ada@example.com', '2024-02-01')",
    )
    .await
    .unwrap();

    db.rollback(&project.migrations()).await.unwrap();
    assert!(!db.schema().await.tables.contains_key("live_accounts"));
}
on_every_backend!(codegen_creates_partial_indexes);
