use ash_core::{ActionDef, AttrType, AttributeDef, IdentityDef, ResourceDef, StatementDef};
use ash_sql::{
    PostgresDialect, SchemaOperation, SqliteDialect, TableSnapshot, diff_snapshots,
    diff_snapshots_with_renames, diff_tables,
};

static RES_V1_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("email", AttrType::String),
];

static RES_V1: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: RES_V1_ATTRS,
    relationships: &[],
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[IdentityDef::new("unique_email", &["email"])],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static RES_V2_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("email", AttrType::String),
    AttributeDef::optional("age", AttrType::Integer),
];

static RES_V2: ResourceDef = ResourceDef {
    name: "Post",
    table: "posts",
    attributes: RES_V2_ATTRS,
    relationships: &[],
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[test]
fn test_snapshot_from_resource_and_json_roundtrip() {
    let snap_pg = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    assert_eq!(snap_pg.table, "users");
    assert_eq!(snap_pg.columns.len(), 2);
    assert_eq!(snap_pg.columns[0].name, "id");
    assert_eq!(snap_pg.columns[0].sql_type, "UUID");
    assert!(snap_pg.columns[0].is_primary_key);
    assert_eq!(snap_pg.columns[1].name, "email");
    assert_eq!(snap_pg.columns[1].sql_type, "TEXT");
    assert!(!snap_pg.columns[1].nullable);
    assert_eq!(snap_pg.identities.len(), 1);

    // JSON round-trip
    let json = serde_json::to_string(&snap_pg).unwrap();
    let deserialized: TableSnapshot = serde_json::from_str(&json).unwrap();
    assert_eq!(snap_pg, deserialized);

    let snap_sqlite = TableSnapshot::from_resource(&RES_V1, &SqliteDialect);
    assert_eq!(snap_sqlite.columns[0].sql_type, "TEXT");
}

#[test]
fn test_diff_create_and_drop_table() {
    let snap_v1 = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);

    // Initial create
    let ops = diff_snapshots(None, Some(&snap_v1));
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], SchemaOperation::CreateTable(t) if t.table == "users"));

    // Drop table
    let ops_drop = diff_snapshots(Some(&snap_v1), None);
    assert_eq!(ops_drop.len(), 1);
    assert!(matches!(&ops_drop[0], SchemaOperation::DropTable(name) if name == "users"));
}

#[test]
fn test_diff_add_column_and_drop_identity() {
    let snap_v1 = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let snap_v2 = TableSnapshot::from_resource(&RES_V2, &PostgresDialect);

    let ops = diff_snapshots(Some(&snap_v1), Some(&snap_v2));
    assert_eq!(ops.len(), 2);

    // Should have AddColumn for `age`
    let has_add_age = ops
        .iter()
        .any(|op| matches!(op, SchemaOperation::AddColumn { column, .. } if column.name == "age"));
    assert!(has_add_age);

    // Should have DropIdentity for `idx_users_unique_email`
    let has_drop_id = ops.iter().any(|op| {
        matches!(op, SchemaOperation::DropIdentity { name, .. } if name == "idx_users_unique_email")
    });
    assert!(has_drop_id);
}

#[test]
fn test_diff_column_modifications() {
    let old = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let mut new = old.clone();

    // Change type and nullability of email
    new.columns[1].sql_type = "VARCHAR(255)".to_string();
    new.columns[1].nullable = true;
    new.columns[1].default = Some("'guest'".to_string());

    let ops = diff_snapshots(Some(&old), Some(&new));
    assert_eq!(ops.len(), 3);

    assert!(ops.contains(&SchemaOperation::AlterColumnType {
        table: "users".into(),
        column: "email".into(),
        old_type: "TEXT".into(),
        new_type: "VARCHAR(255)".into(),
    }));

    assert!(ops.contains(&SchemaOperation::SetNullable {
        table: "users".into(),
        column: "email".into(),
        nullable: true,
    }));

    assert!(ops.contains(&SchemaOperation::SetDefault {
        table: "users".into(),
        column: "email".into(),
        default: Some("'guest'".into()),
    }));
}

#[test]
fn test_diff_rename_column() {
    let old = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let mut new = old.clone();
    new.columns[1].name = "contact_email".to_string();

    let ops = diff_snapshots_with_renames(Some(&old), Some(&new), &[("email", "contact_email")]);

    assert_eq!(ops.len(), 1);
    assert_eq!(
        ops[0],
        SchemaOperation::RenameColumn {
            table: "users".into(),
            old_name: "email".into(),
            new_name: "contact_email".into(),
        }
    );
}

#[test]
fn test_diff_tables_multiple() {
    let t1 = TableSnapshot::from_resource(&RES_V1, &PostgresDialect);
    let t2 = TableSnapshot::from_resource(&RES_V2, &PostgresDialect);

    let old_tables = vec![t1.clone()];
    let new_tables = vec![t1, t2];
    let ops = diff_tables(&old_tables, &new_tables);
    // The second table was created
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], SchemaOperation::CreateTable(_)));
}

#[test]
fn custom_statements_are_dialect_filtered_and_ordered_around_the_table() {
    static ATTRS: &[AttributeDef] = &[AttributeDef::uuid_pk("id")];
    static STATEMENTS: &[StatementDef] = &[
        StatementDef {
            name: "sidecar",
            dialects: &[],
            up: "CREATE TABLE users_sidecar (id TEXT PRIMARY KEY)",
            down: "DROP TABLE IF EXISTS users_sidecar",
        },
        StatementDef {
            name: "citext",
            dialects: &["postgres"],
            up: "CREATE EXTENSION IF NOT EXISTS citext",
            down: "DROP EXTENSION IF EXISTS citext",
        },
    ];
    let mut resource = RES_V1;
    resource.attributes = ATTRS;
    resource.identities = &[];
    resource.statements = STATEMENTS;

    let sqlite = TableSnapshot::from_resource(&resource, &SqliteDialect);
    assert_eq!(
        sqlite
            .statements
            .iter()
            .map(|statement| statement.name.as_str())
            .collect::<Vec<_>>(),
        vec!["sidecar"]
    );
    let postgres = TableSnapshot::from_resource(&resource, &PostgresDialect);
    assert_eq!(postgres.statements.len(), 2);

    let ops = diff_snapshots(None, Some(&postgres));
    assert!(matches!(
        &ops[0],
        SchemaOperation::RunStatement { statement, .. } if statement.name == "sidecar"
    ));
    assert!(matches!(
        &ops[1],
        SchemaOperation::RunStatement { statement, .. } if statement.name == "citext"
    ));
    assert!(matches!(&ops[2], SchemaOperation::CreateTable(_)));

    let files = ash_sql::generate_migration(&PostgresDialect, "create_users", &ops);
    let sidecar = files.up_sql.find("CREATE TABLE users_sidecar").unwrap();
    let extension = files
        .up_sql
        .find("CREATE EXTENSION IF NOT EXISTS citext;")
        .unwrap();
    let table = files
        .up_sql
        .find("CREATE TABLE IF NOT EXISTS \"users\"")
        .unwrap();
    assert!(sidecar < extension && extension < table);
    let drop_table = files
        .down_sql
        .find("DROP TABLE IF EXISTS \"users\"")
        .unwrap();
    let drop_ext = files
        .down_sql
        .find("DROP EXTENSION IF EXISTS citext;")
        .unwrap();
    let drop_side = files
        .down_sql
        .find("DROP TABLE IF EXISTS users_sidecar;")
        .unwrap();
    assert!(drop_table < drop_ext && drop_ext < drop_side);

    let sqlite_files = ash_sql::generate_migration(
        &SqliteDialect,
        "create_users",
        &diff_snapshots(None, Some(&sqlite)),
    );
    assert!(!sqlite_files.up_sql.contains("citext"));
    assert!(!sqlite_files.down_sql.contains("citext"));

    let mut changed = postgres.clone();
    changed.statements[0].up = "CREATE TABLE users_sidecar (id TEXT)".into();
    let changed_ops = diff_snapshots(Some(&postgres), Some(&changed));
    assert_eq!(changed_ops.len(), 1);
    assert!(matches!(
        &changed_ops[0],
        SchemaOperation::RunStatement { statement, .. } if statement.up.contains("(id TEXT)")
    ));

    let mut bare = postgres.clone();
    bare.statements.clear();
    let removed = diff_snapshots(Some(&postgres), Some(&bare));
    assert!(matches!(
        removed.last(),
        Some(SchemaOperation::DropStatement { name, .. }) if name == "citext"
    ));

    let mut json: serde_json::Value = serde_json::to_value(&sqlite).unwrap();
    json.as_object_mut().unwrap().remove("statements");
    let loaded: TableSnapshot = serde_json::from_value(json).unwrap();
    assert!(loaded.statements.is_empty());
}
