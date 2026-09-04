use ash_core::{
    ActionDef, AttrType, AttributeDef, CompiledQuery, DataLayer, Error, FieldMap, Filter,
    IdentityDef, ResourceDef, TransactionSupport, Value,
};
use ash_postgres::Postgres;
use ash_sql::{generate_migration_with_version, PostgresDialect, TableSnapshot};
use uuid::Uuid;

static CUSTOMER_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("email", AttrType::String),
    AttributeDef::optional("name", AttrType::String),
    AttributeDef::optional("score", AttrType::Integer),
    AttributeDef::optional("is_active", AttrType::Boolean),
    AttributeDef::optional("version", AttrType::Integer),
];

static CUSTOMER_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["email", "name", "score", "is_active"]),
    ActionDef::read("read").primary(),
    ActionDef::update("update").accept(&["email", "name", "score", "is_active"]),
    ActionDef::destroy("destroy"),
];

static CUSTOMER_IDENTS: &[IdentityDef] = &[IdentityDef::new("unique_email", &["email"])];

static CUSTOMER_DEF: ResourceDef = ResourceDef {
    name: "Customer",
    table: "customers",
    attributes: CUSTOMER_ATTRS,
    relationships: &[],
    actions: CUSTOMER_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: CUSTOMER_IDENTS,
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

async fn get_test_postgres() -> Option<Postgres> {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".to_string());
    Postgres::connect(&url).await.ok()
}

#[tokio::test]
async fn test_postgres_crud_and_returning_and_upsert() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };

    // Clean up table if exists from previous test
    let _ = pg.install(&[&CUSTOMER_DEF]).await;

    let id = Uuid::new_v4();
    let email = format!("user_{}@example.com", Uuid::new_v4().simple());

    // 1. CREATE with RETURNING * (single roundtrip)
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("email".into(), Value::String(email.clone()));
    fields.insert("name".into(), Value::String("Alice".into()));
    fields.insert("score".into(), Value::Int(100));
    fields.insert("is_active".into(), Value::Bool(true));

    let created = pg.create(&CUSTOMER_DEF, id, fields).await.expect("Create failed");
    assert_eq!(created.get("id"), Some(&Value::Uuid(id)));
    assert_eq!(created.get("email"), Some(&Value::String(email.clone())));
    assert_eq!(created.get("name"), Some(&Value::String("Alice".into())));
    assert_eq!(created.get("score"), Some(&Value::Int(100)));
    assert_eq!(created.get("is_active"), Some(&Value::Bool(true)));

    // 2. READ via run_query with filter
    let query = CompiledQuery {
        filter: Some(Filter::eq("email", email.clone())),
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&CUSTOMER_DEF, &query).await.expect("Query failed");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("name"), Some(&Value::String("Alice".into())));

    // 3. UPDATE with RETURNING *
    let mut update_fields = FieldMap::new();
    update_fields.insert("name".into(), Value::String("Alice Updated".into()));
    update_fields.insert("score".into(), Value::Int(250));

    let updated = pg.update(&CUSTOMER_DEF, id, update_fields).await.expect("Update failed");
    assert_eq!(updated.get("name"), Some(&Value::String("Alice Updated".into())));
    assert_eq!(updated.get("score"), Some(&Value::Int(250)));

    // 4. UPSERT on conflict DO UPDATE
    let mut upsert_fields = FieldMap::new();
    upsert_fields.insert("id".into(), Value::Uuid(Uuid::new_v4()));
    upsert_fields.insert("email".into(), Value::String(email.clone()));
    upsert_fields.insert("name".into(), Value::String("Alice Upserted".into()));
    upsert_fields.insert("score".into(), Value::Int(300));

    let upserted = pg
        .upsert(
            &CUSTOMER_DEF,
            id,
            upsert_fields,
            &CUSTOMER_IDENTS[0],
            &["name".to_string(), "score".to_string()],
        )
        .await
        .expect("Upsert failed");
    assert_eq!(upserted.get("name"), Some(&Value::String("Alice Upserted".into())));
    assert_eq!(upserted.get("score"), Some(&Value::Int(300)));

    // 5. DESTROY
    pg.destroy(&CUSTOMER_DEF, id).await.expect("Destroy failed");

    // Verify row is gone
    let rows_after = pg.run_query(&CUSTOMER_DEF, &query).await.expect("Query failed");
    assert!(rows_after.is_empty());
}

#[tokio::test]
async fn test_postgres_unique_violation_error_mapping() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let _ = pg.install(&[&CUSTOMER_DEF]).await;

    let email = format!("dup_{}@example.com", Uuid::new_v4().simple());

    let mut fields1 = FieldMap::new();
    let id1 = Uuid::new_v4();
    fields1.insert("id".into(), Value::Uuid(id1));
    fields1.insert("email".into(), Value::String(email.clone()));
    pg.create(&CUSTOMER_DEF, id1, fields1).await.unwrap();

    let mut fields2 = FieldMap::new();
    let id2 = Uuid::new_v4();
    fields2.insert("id".into(), Value::Uuid(id2));
    fields2.insert("email".into(), Value::String(email.clone()));

    let err = pg.create(&CUSTOMER_DEF, id2, fields2).await.unwrap_err();
    match err {
        Error::IdentityConflict { identity, .. } => {
            assert_eq!(identity, "unique_email");
        }
        other => panic!("Expected IdentityConflict, got {other:?}"),
    }

    // Cleanup
    let _ = pg.destroy(&CUSTOMER_DEF, id1).await;
}

#[tokio::test]
async fn test_postgres_transaction_commit_and_rollback() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let _ = pg.install(&[&CUSTOMER_DEF]).await;

    let email_rollback = format!("rollback_{}@example.com", Uuid::new_v4().simple());
    let id_rollback = Uuid::new_v4();

    // 1. Transaction that fails and rolls back
    let res: Result<(), Error> = pg
        .transaction(|tx| {
            let tx = tx.clone();
            let email = email_rollback.clone();
            async move {
                let mut fields = FieldMap::new();
                fields.insert("id".into(), Value::Uuid(id_rollback));
                fields.insert("email".into(), Value::String(email));
                tx.create(&CUSTOMER_DEF, id_rollback, fields).await?;
                // Force error to trigger rollback
                Err(Error::Invalid("abort transaction".into()))
            }
        })
        .await;

    assert!(res.is_err());

    // Verify record was rolled back and does not exist
    let query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(id_rollback))),
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&CUSTOMER_DEF, &query).await.unwrap();
    assert!(rows.is_empty(), "Record should not exist after rollback");

    // 2. Transaction that succeeds and commits
    let email_commit = format!("commit_{}@example.com", Uuid::new_v4().simple());
    let id_commit = Uuid::new_v4();

    pg.transaction(|tx| {
        let tx = tx.clone();
        let email = email_commit.clone();
        async move {
            let mut fields = FieldMap::new();
            fields.insert("id".into(), Value::Uuid(id_commit));
            fields.insert("email".into(), Value::String(email));
            tx.create(&CUSTOMER_DEF, id_commit, fields).await?;
            Ok(())
        }
    })
    .await
    .unwrap();

    let query_commit = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(id_commit))),
        ..CompiledQuery::default()
    };
    let rows_commit = pg.run_query(&CUSTOMER_DEF, &query_commit).await.unwrap();
    assert_eq!(rows_commit.len(), 1);

    // Cleanup
    let _ = pg.destroy(&CUSTOMER_DEF, id_commit).await;
}

#[tokio::test]
async fn test_postgres_declarative_migration_runner() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let pool = pg.pool().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("ash_pg_migrations_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let version = format!("2026{}", &Uuid::new_v4().simple().to_string()[..10]);
    let snap = TableSnapshot::from_resource(&CUSTOMER_DEF, &PostgresDialect);
    let files = generate_migration_with_version(&PostgresDialect, &version, "create_customers", &[
        ash_sql::SchemaOperation::CreateTable(snap),
    ]);

    let up_file = temp_dir.join(&files.up_filename);
    let down_file = temp_dir.join(&files.down_filename);
    std::fs::write(&up_file, &files.up_sql).unwrap();
    std::fs::write(&down_file, &files.down_sql).unwrap();

    let applied = ash_postgres::migrate(pool, &temp_dir).await.expect("Migration failed");
    assert_eq!(applied, vec![version.clone()]);

    // Running again should find 0 pending
    let applied_again = ash_postgres::migrate(pool, &temp_dir).await.expect("Second migration check failed");
    assert!(applied_again.is_empty());

    // Cleanup
    let _ = sqlx::query("DELETE FROM _ash_schema_migrations WHERE version = $1")
        .bind(&version)
        .execute(pool)
        .await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_postgres_in_query_large_batch_any_array() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };

    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    let mut f1 = ash_core::FieldMap::new();
    f1.insert("id".into(), Value::Uuid(id1));
    f1.insert("email".into(), Value::String(format!("user1_{}@example.com", Uuid::new_v4().simple())));
    f1.insert("name".into(), Value::String("User 1".into()));
    f1.insert("role".into(), Value::String("customer".into()));
    f1.insert("active".into(), Value::Bool(true));
    f1.insert("points".into(), Value::Int(10));
    pg.create(&CUSTOMER_DEF, id1, f1).await.unwrap();

    let mut f2 = ash_core::FieldMap::new();
    f2.insert("id".into(), Value::Uuid(id2));
    f2.insert("email".into(), Value::String(format!("user2_{}@example.com", Uuid::new_v4().simple())));
    f2.insert("name".into(), Value::String("User 2".into()));
    f2.insert("role".into(), Value::String("customer".into()));
    f2.insert("active".into(), Value::Bool(true));
    f2.insert("points".into(), Value::Int(20));
    pg.create(&CUSTOMER_DEF, id2, f2).await.unwrap();

    // Large list with 2,000 UUIDs
    let mut large_ids = vec![id1, id2];
    for _ in 0..2000 {
        large_ids.push(Uuid::new_v4());
    }

    let id_values: Vec<Value> = large_ids.into_iter().map(Value::Uuid).collect();
    let query = ash_core::CompiledQuery {
        filter: Some(ash_core::Filter::in_list("id", id_values)),
        ..ash_core::CompiledQuery::default()
    };

    let rows = pg.run_query(&CUSTOMER_DEF, &query).await.unwrap();
    assert_eq!(rows.len(), 2);

    let _ = pg.destroy(&CUSTOMER_DEF, id1).await;
    let _ = pg.destroy(&CUSTOMER_DEF, id2).await;
}

static NODE_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::optional("parent_id", AttrType::Uuid),
];

static NODE_RELS: &[ash_core::RelationshipDef] = &[
    ash_core::RelationshipDef::has_many("children", || &NODE_DEF, "parent_id"),
];

static NODE_AGGS: &[ash_core::AggregateDef] = &[
    ash_core::AggregateDef::count("children_count", "children"),
];

static NODE_DEF: ResourceDef = ResourceDef {
    name: "Node",
    table: "nodes",
    attributes: NODE_ATTRS,
    relationships: NODE_RELS,
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: NODE_AGGS,
    extensions: &[],
    notifiers: &[],
    identities: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

#[tokio::test]
async fn test_postgres_self_referential_aggregate() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let pool = pg.pool().unwrap();

    let _ = sqlx::query("DROP TABLE IF EXISTS nodes;").execute(pool).await;
    let _ = sqlx::query("CREATE TABLE nodes (id UUID PRIMARY KEY, name TEXT NOT NULL, parent_id UUID REFERENCES nodes(id));")
        .execute(pool)
        .await
        .unwrap();

    let root_id = Uuid::new_v4();
    let mut root_fields = ash_core::FieldMap::new();
    root_fields.insert("id".into(), Value::Uuid(root_id));
    root_fields.insert("name".into(), Value::String("Root Node".into()));
    pg.create(&NODE_DEF, root_id, root_fields).await.unwrap();

    for i in 1..=3 {
        let child_id = Uuid::new_v4();
        let mut child_fields = ash_core::FieldMap::new();
        child_fields.insert("id".into(), Value::Uuid(child_id));
        child_fields.insert("name".into(), Value::String(format!("Child Node {i}")));
        child_fields.insert("parent_id".into(), Value::Uuid(root_id));
        pg.create(&NODE_DEF, child_id, child_fields).await.unwrap();
    }

    let query = ash_core::CompiledQuery {
        filter: Some(ash_core::Filter::eq("id", Value::Uuid(root_id))),
        aggregates: vec!["children_count".to_string()],
        ..ash_core::CompiledQuery::default()
    };

    let rows = pg.run_query(&NODE_DEF, &query).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("children_count"), Some(&Value::Int(3)));

    let _ = sqlx::query("DROP TABLE IF EXISTS nodes;").execute(pool).await;
}
