use ash_core::{ActionDef, AttrType, AttributeDef, DataLayer, FieldMap, ResourceDef, Result, Value};
use ash_sql::MigrationExecutor;
use ash_sqlite::Sqlite;
use uuid::Uuid;

const TICKET: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("subject", AttrType::String),
        AttributeDef::required(
            "status",
            AttrType::Atom {
                one_of: &["open", "closed"],
            },
        ),
        AttributeDef::optional("priority", AttrType::Integer),
    ],
    relationships: &[],
    actions: &[
        ActionDef::create("create").primary(),
        ActionDef::read("read").primary(),
    ],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "DefaultStore",
};

#[tokio::test]
async fn test_sqlite_crud_and_install() -> Result<()> {
    let db = Sqlite::memory().await?;
    db.install(&[&TICKET]).await?;

    let id = Uuid::new_v4();
    let mut fields = FieldMap::new();
    fields.insert("id".to_string(), Value::Uuid(id));
    fields.insert("subject".to_string(), Value::String("Printer broken".into()));
    fields.insert("status".to_string(), Value::String("open".into()));
    fields.insert("priority".to_string(), Value::Int(1));

    let created = db.create(&TICKET, id, fields).await?;
    assert_eq!(created.get("subject"), Some(&Value::String("Printer broken".into())));

    let query = ash_core::CompiledQuery::default();
    let rows = db.run_query(&TICKET, &query).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("id"), Some(&Value::Uuid(id)));

    let mut update_fields = FieldMap::new();
    update_fields.insert("status".to_string(), Value::String("closed".into()));
    let updated = db.update(&TICKET, id, update_fields).await?;
    assert_eq!(updated.get("status"), Some(&Value::String("closed".into())));

    db.destroy(&TICKET, id).await?;
    let rows_after = db.run_query(&TICKET, &query).await?;
    assert_eq!(rows_after.len(), 0);

    Ok(())
}

#[tokio::test]
async fn test_sqlite_declarative_migration_and_rollback() -> Result<()> {
    let temp_dir = std::env::temp_dir().join(format!("ash_sqlite_test_{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let v1 = "20260903000001_create_items";
    let up_1 = format!("{temp_dir}/{v1}.up.sql", temp_dir = temp_dir.display());
    let down_1 = format!("{temp_dir}/{v1}.down.sql", temp_dir = temp_dir.display());

    std::fs::write(&up_1, "CREATE TABLE items (id TEXT PRIMARY KEY, name TEXT);").unwrap();
    std::fs::write(&down_1, "DROP TABLE items;").unwrap();

    let db = Sqlite::memory().await?;
    let applied = db.migrate(&temp_dir).await?;
    assert_eq!(applied, vec!["20260903000001"]);

    // Verify table exists by inserting
    db.execute_script("INSERT INTO items (id, name) VALUES ('1', 'Widget');").await?;

    // Rollback
    let rolled_back = db.rollback(&temp_dir).await?;
    assert_eq!(rolled_back, Some("20260903000001".to_string()));

    // Verify table no longer exists
    let err = db.execute_script("SELECT * FROM items;").await;
    assert!(err.is_err());

    let _ = std::fs::remove_dir_all(&temp_dir);
    Ok(())
}
