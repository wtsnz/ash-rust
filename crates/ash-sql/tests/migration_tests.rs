use ash_core::{
    ActionDef, AttrType, AttributeDef, IdentityDef, IndexDef, RelationshipDef, ResourceDef,
};
use ash_sql::{
    MemoryMigrationExecutor, Migrator, PostgresDialect, SqliteDialect, TableSnapshot,
    diff_snapshots, generate_migration_with_version,
};

static RES_V1_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("username", AttrType::String),
];

static RES_V1: ResourceDef = ResourceDef {
    name: "Account",
    table: "accounts",
    attributes: RES_V1_ATTRS,
    relationships: &[],
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[IdentityDef::new("unique_username", &["username"])],
    indexes: &[],
    checks: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[test]
fn test_generate_migration_sql_postgres_and_sqlite() {
    let snap_pg = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let ops_pg = diff_snapshots(None, Some(&snap_pg));

    let files_pg = generate_migration_with_version(
        &PostgresDialect,
        "20260903000001",
        "create_accounts",
        &ops_pg,
    );
    assert_eq!(
        files_pg.up_filename,
        "20260903000001_create_accounts.postgres.up.sql"
    );
    assert_eq!(
        files_pg.down_filename,
        "20260903000001_create_accounts.postgres.down.sql"
    );

    assert!(
        files_pg
            .up_sql
            .contains("CREATE TABLE IF NOT EXISTS \"accounts\"")
    );
    assert!(files_pg.up_sql.contains("\"id\" UUID PRIMARY KEY"));
    assert!(files_pg.up_sql.contains("CREATE UNIQUE INDEX IF NOT EXISTS \"idx_accounts_unique_username\" ON \"accounts\" (\"username\");"));
    assert!(
        files_pg
            .down_sql
            .contains("DROP TABLE IF EXISTS \"accounts\";")
    );

    let snap_sqlite = TableSnapshot::from_resource(&RES_V1, &SqliteDialect);
    let ops_sqlite = diff_snapshots(None, Some(&snap_sqlite));

    let files_sqlite = generate_migration_with_version(
        &SqliteDialect,
        "20260903000001",
        "create_accounts",
        &ops_sqlite,
    );
    assert_eq!(
        files_sqlite.up_filename,
        "20260903000001_create_accounts.sqlite.up.sql"
    );
    assert!(files_sqlite.up_sql.contains("\"id\" TEXT PRIMARY KEY"));
}

#[test]
fn unique_index_nulls_not_distinct_is_postgres_only() {
    static NULLS_IDENT: &[IdentityDef] =
        &[IdentityDef::new("unique_username", &["username"]).with_nils_distinct(false)];
    let mut resource = RES_V1;
    resource.identities = NULLS_IDENT;
    let postgres = TableSnapshot::from_resource(&resource, &PostgresDialect);
    let postgres_sql = generate_migration_with_version(
        &PostgresDialect,
        "20260903000002",
        "create_accounts",
        &diff_snapshots(None, Some(&postgres)),
    );
    assert!(
        postgres_sql.up_sql.contains(
            "CREATE UNIQUE INDEX IF NOT EXISTS \"idx_accounts_unique_username\" ON \"accounts\" (\"username\") NULLS NOT DISTINCT;"
        )
    );
    let sqlite = TableSnapshot::from_resource(&resource, &SqliteDialect);
    let sqlite_sql = generate_migration_with_version(
        &SqliteDialect,
        "20260903000002",
        "create_accounts",
        &diff_snapshots(None, Some(&sqlite)),
    );
    assert!(sqlite_sql
        .up_sql
        .contains("CREATE UNIQUE INDEX IF NOT EXISTS \"idx_accounts_unique_username\" ON \"accounts\" (\"username\");"));
    assert!(!sqlite_sql.up_sql.contains("NULLS NOT DISTINCT"));
}

#[test]
fn index_using_is_emitted_for_postgres_only() {
    static INDEXES: &[IndexDef] = &[IndexDef::new("by_body", &["username"]).with_method("gin")];
    let mut resource = RES_V1;
    resource.identities = &[];
    resource.indexes = INDEXES;
    let postgres = generate_migration_with_version(
        &PostgresDialect,
        "20260903000003",
        "create_accounts",
        &diff_snapshots(
            None,
            Some(&TableSnapshot::from_resource(&resource, &PostgresDialect)),
        ),
    );
    assert!(postgres.up_sql.contains(
        "CREATE INDEX IF NOT EXISTS \"idx_accounts_by_body\" ON \"accounts\" USING gin (\"username\");"
    ));
    let sqlite = generate_migration_with_version(
        &SqliteDialect,
        "20260903000003",
        "create_accounts",
        &diff_snapshots(
            None,
            Some(&TableSnapshot::from_resource(&resource, &SqliteDialect)),
        ),
    );
    assert!(sqlite.up_sql.contains(
        "CREATE INDEX IF NOT EXISTS \"idx_accounts_by_body\" ON \"accounts\" (\"username\");"
    ));
    assert!(!sqlite.up_sql.contains("USING"));
}

#[test]
fn old_reference_snapshot_keeps_a_single_column_key() {
    let json = r#"{
        "name": "fk_tickets_org",
        "column": "org_id",
        "target_table": "orgs",
        "target_column": "id",
        "on_delete": "CASCADE"
    }"#;
    let loaded: ash_sql::ReferenceSnapshot = serde_json::from_str(json).unwrap();
    let fresh = ash_sql::ReferenceSnapshot {
        name: "fk_tickets_org".into(),
        column: "org_id".into(),
        columns: vec!["org_id".into()],
        target_table: "orgs".into(),
        target_column: "id".into(),
        target_columns: vec!["id".into()],
        on_delete: "CASCADE".into(),
        on_update: "NO ACTION".into(),
    };
    assert_eq!(loaded, fresh);
}

#[test]
fn composite_foreign_key_lists_every_column() {
    static PARENT_ATTRS: &[AttributeDef] = &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("tenant_id", AttrType::Uuid),
        AttributeDef::required("code", AttrType::String),
    ];
    static PARENT: ResourceDef = ResourceDef {
        name: "Account",
        table: "accounts",
        attributes: PARENT_ATTRS,
        relationships: &[],
        actions: &[],
        policies: &[],
        field_policies: &[],
        calculations: &[],
        aggregates: &[],
        extensions: &[],
        notifiers: &[],
        identities: &[IdentityDef::new("tenant_code", &["tenant_id", "code"])],
        indexes: &[],
        checks: &[],
        embedded: false,
        data_layer: ash_core::DataLayerKind::Postgres,
        timestamps: None,
        store_type_id: ash_core::default_store_type_id,
        store_name: "default",
        multitenancy: None,
    };
    static CHILD_ATTRS: &[AttributeDef] = &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("tenant_id", AttrType::Uuid),
        AttributeDef::required("code", AttrType::String),
    ];
    static CHILD_RELS: &[RelationshipDef] = &[RelationshipDef::belongs_to(
        "account",
        || &PARENT,
        "tenant_id",
    )
    .with_keys(&["tenant_id", "code"], &["tenant_id", "code"])];
    static CHILD: ResourceDef = ResourceDef {
        name: "Membership",
        table: "memberships",
        attributes: CHILD_ATTRS,
        relationships: CHILD_RELS,
        actions: &[],
        policies: &[],
        field_policies: &[],
        calculations: &[],
        aggregates: &[],
        extensions: &[],
        notifiers: &[],
        identities: &[],
        indexes: &[],
        checks: &[],
        embedded: false,
        data_layer: ash_core::DataLayerKind::Postgres,
        timestamps: None,
        store_type_id: ash_core::default_store_type_id,
        store_name: "default",
        multitenancy: None,
    };
    let sql = generate_migration_with_version(
        &PostgresDialect,
        "20260903000004",
        "create_memberships",
        &diff_snapshots(
            None,
            Some(&TableSnapshot::from_resource(&CHILD, &PostgresDialect)),
        ),
    );
    assert!(sql.up_sql.contains(
        "FOREIGN KEY (\"tenant_id\", \"code\") REFERENCES \"accounts\" (\"tenant_id\", \"code\") ON DELETE NO ACTION"
    ));
}

#[test]
fn atom_one_of_becomes_a_named_check() {
    static ATTRS: &[AttributeDef] = &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required(
            "status",
            AttrType::Atom {
                one_of: &["open", "closed"],
            },
        ),
    ];
    let mut resource = RES_V1;
    resource.attributes = ATTRS;
    resource.identities = &[];
    let sql = generate_migration_with_version(
        &SqliteDialect,
        "20260903000005",
        "create_accounts",
        &diff_snapshots(
            None,
            Some(&TableSnapshot::from_resource(&resource, &SqliteDialect)),
        ),
    );
    assert!(sql.up_sql.contains(
        "CONSTRAINT \"ck_accounts_status_one_of\" CHECK (\"status\" IN ('open', 'closed'))"
    ));
}

#[tokio::test]
async fn test_migrator_run_and_rollback_lifecycle() {
    let temp_dir =
        std::env::temp_dir().join(format!("ash_test_migrations_{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let snap = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let ops = diff_snapshots(None, Some(&snap));
    let files = generate_migration_with_version(
        &PostgresDialect,
        "20260903100000",
        "create_accounts",
        &ops,
    );

    let up_file = temp_dir.join(&files.up_filename);
    let down_file = temp_dir.join(&files.down_filename);
    std::fs::write(&up_file, &files.up_sql).unwrap();
    std::fs::write(&down_file, &files.down_sql).unwrap();

    let migrator = Migrator::new(PostgresDialect, &temp_dir);
    let executor = MemoryMigrationExecutor::new();

    // 1. Discover migrations
    let discovered = migrator.discover_migrations().unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(discovered[0].version, "20260903100000");

    // 2. Pending migrations
    let pending = migrator.pending_migrations(&executor).await.unwrap();
    assert_eq!(pending.len(), 1);

    // 3. Run migrations
    let applied = migrator.run(&executor).await.unwrap();
    assert_eq!(applied, vec!["20260903100000"]);

    let pending_after = migrator.pending_migrations(&executor).await.unwrap();
    assert!(pending_after.is_empty());

    {
        let executed_scripts = executor.executed_scripts.lock().unwrap();
        assert_eq!(executed_scripts.len(), 1);
        assert!(executed_scripts[0].contains("CREATE TABLE IF NOT EXISTS \"accounts\""));
    }

    // 4. Rollback
    let rolled_back = migrator.rollback(&executor).await.unwrap();
    assert_eq!(rolled_back, Some("20260903100000".to_string()));

    let pending_after_rollback = migrator.pending_migrations(&executor).await.unwrap();
    assert_eq!(pending_after_rollback.len(), 1);

    {
        let executed_after_rollback = executor.executed_scripts.lock().unwrap();
        assert_eq!(executed_after_rollback.len(), 2);
        assert!(executed_after_rollback[1].contains("DROP TABLE IF EXISTS \"accounts\""));
    }

    // Cleanup
    let _ = std::fs::remove_dir_all(&temp_dir);
}
