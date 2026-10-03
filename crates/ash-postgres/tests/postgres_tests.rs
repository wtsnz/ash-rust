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

    let created = pg.create(&CUSTOMER_DEF, None, id, fields).await.expect("Create failed");
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

    let updated = pg.update(&CUSTOMER_DEF, None, id, update_fields).await.expect("Update failed");
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
            &CUSTOMER_DEF, None,
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
    pg.destroy(&CUSTOMER_DEF, None, id).await.expect("Destroy failed");

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
    pg.create(&CUSTOMER_DEF, None, id1, fields1).await.unwrap();

    let mut fields2 = FieldMap::new();
    let id2 = Uuid::new_v4();
    fields2.insert("id".into(), Value::Uuid(id2));
    fields2.insert("email".into(), Value::String(email.clone()));

    let err = pg.create(&CUSTOMER_DEF, None, id2, fields2).await.unwrap_err();
    match err {
        Error::IdentityConflict { identity, .. } => {
            assert_eq!(identity, "unique_email");
        }
        other => panic!("Expected IdentityConflict, got {other:?}"),
    }

    // Cleanup
    let _ = pg.destroy(&CUSTOMER_DEF, None, id1).await;
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
                tx.create(&CUSTOMER_DEF, None, id_rollback, fields).await?;
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
            tx.create(&CUSTOMER_DEF, None, id_commit, fields).await?;
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
    let _ = pg.destroy(&CUSTOMER_DEF, None, id_commit).await;
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
    let table = format!("customers_mig_{}", &Uuid::new_v4().simple().to_string()[..8]);
    let mut snap = TableSnapshot::from_resource(&CUSTOMER_DEF, &PostgresDialect);
    snap.table = table.clone();
    for identity in &mut snap.identities {
        identity.name = format!("{table}_{}", identity.name);
    }
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
    let _ = pg.execute_sql(&format!("DELETE FROM _ash_schema_migrations WHERE version = '{version}'")).await;
    let _ = pg.execute_sql(&format!("DROP TABLE IF EXISTS {table}")).await;
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn test_postgres_in_query_large_batch_any_array() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let _ = pg.install(&[&CUSTOMER_DEF]).await;

    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();
    let mut f1 = ash_core::FieldMap::new();
    f1.insert("id".into(), Value::Uuid(id1));
    f1.insert("email".into(), Value::String(format!("user1_{}@example.com", Uuid::new_v4().simple())));
    f1.insert("name".into(), Value::String("User 1".into()));
    f1.insert("role".into(), Value::String("customer".into()));
    f1.insert("active".into(), Value::Bool(true));
    f1.insert("points".into(), Value::Int(10));
    pg.create(&CUSTOMER_DEF, None, id1, f1).await.unwrap();

    let mut f2 = ash_core::FieldMap::new();
    f2.insert("id".into(), Value::Uuid(id2));
    f2.insert("email".into(), Value::String(format!("user2_{}@example.com", Uuid::new_v4().simple())));
    f2.insert("name".into(), Value::String("User 2".into()));
    f2.insert("role".into(), Value::String("customer".into()));
    f2.insert("active".into(), Value::Bool(true));
    f2.insert("points".into(), Value::Int(20));
    pg.create(&CUSTOMER_DEF, None, id2, f2).await.unwrap();

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

    let _ = pg.destroy(&CUSTOMER_DEF, None, id1).await;
    let _ = pg.destroy(&CUSTOMER_DEF, None, id2).await;
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

#[tokio::test]
async fn test_postgres_self_referential_aggregate() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };

    let _ = pg.execute_sql("DROP TABLE IF EXISTS nodes;").await;
    pg.execute_sql("CREATE TABLE nodes (id UUID PRIMARY KEY, name TEXT NOT NULL, parent_id UUID REFERENCES nodes(id));")
        .await
        .unwrap();

    let root_id = Uuid::new_v4();
    let mut root_fields = ash_core::FieldMap::new();
    root_fields.insert("id".into(), Value::Uuid(root_id));
    root_fields.insert("name".into(), Value::String("Root Node".into()));
    pg.create(&NODE_DEF, None, root_id, root_fields).await.unwrap();

    for i in 1..=3 {
        let child_id = Uuid::new_v4();
        let mut child_fields = ash_core::FieldMap::new();
        child_fields.insert("id".into(), Value::Uuid(child_id));
        child_fields.insert("name".into(), Value::String(format!("Child Node {i}")));
        child_fields.insert("parent_id".into(), Value::Uuid(root_id));
        pg.create(&NODE_DEF, None, child_id, child_fields).await.unwrap();
    }

    let query = ash_core::CompiledQuery {
        filter: Some(ash_core::Filter::eq("id", Value::Uuid(root_id))),
        aggregates: vec!["children_count".to_string()],
        ..ash_core::CompiledQuery::default()
    };

    let rows = pg.run_query(&NODE_DEF, &query).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get("children_count"), Some(&Value::Int(3)));

    let _ = pg.execute_sql("DROP TABLE IF EXISTS nodes;").await;
}

static DOCUMENT_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::version("version"),
];

static DOCUMENT_DEF: ResourceDef = ResourceDef {
    name: "Document",
    table: "documents",
    attributes: DOCUMENT_ATTRS,
    relationships: &[],
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
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[tokio::test]
async fn test_postgres_optimistic_locking_stale_record() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let _ = pg.execute_sql("DROP TABLE IF EXISTS documents;").await;
    pg.execute_sql("CREATE TABLE documents (id UUID PRIMARY KEY, title TEXT NOT NULL, version BIGINT NOT NULL);")
        .await
        .unwrap();

    let id = Uuid::new_v4();
    let mut fields = ash_core::FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("title".into(), Value::String("Version 1".into()));
    fields.insert("version".into(), Value::Int(1));
    pg.create(&DOCUMENT_DEF, None, id, fields).await.unwrap();

    // Successful update: version moves from 1 to 2
    let mut update_fields = ash_core::FieldMap::new();
    update_fields.insert("title".into(), Value::String("Version 2".into()));
    update_fields.insert("version".into(), Value::Int(2));
    let res = pg.update(&DOCUMENT_DEF, None, id, update_fields.clone()).await.unwrap();
    assert_eq!(res.get("version"), Some(&Value::Int(2)));

    // Second update with version = 2 (expected 1): fails because current DB version is 2!
    let err = pg.update(&DOCUMENT_DEF, None, id, update_fields).await.unwrap_err();
    assert!(matches!(err, ash_core::Error::StaleRecord { .. }));

    // Non-existent ID: returns NotFound
    let missing_id = Uuid::new_v4();
    let mut missing_fields = ash_core::FieldMap::new();
    missing_fields.insert("title".into(), Value::String("Ghost".into()));
    missing_fields.insert("version".into(), Value::Int(2));
    let err_missing = pg.update(&DOCUMENT_DEF, None, missing_id, missing_fields).await.unwrap_err();
    assert!(matches!(err_missing, ash_core::Error::NotFound));

    let _ = pg.execute_sql("DROP TABLE IF EXISTS documents;").await;
}

#[tokio::test]
async fn test_postgres_empty_in_and_empty_bulk_operations() {
    let Some(pg) = get_test_postgres().await else {
        return;
    };
    let _ = pg.install(&[&CUSTOMER_DEF]).await;

    // 1. Query with Filter::in_list of empty vec -> returns empty vec, no error
    let query_empty = CompiledQuery {
        filter: Some(Filter::in_list("id", Vec::<Value>::new())),
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&CUSTOMER_DEF, &query_empty).await.unwrap();
    assert!(rows.is_empty());

    // 2. Query with NOT (Filter::in_list empty) -> matches without error
    let query_not_empty = CompiledQuery {
        filter: Some(!Filter::in_list("id", Vec::<Value>::new())),
        ..CompiledQuery::default()
    };
    let _ = pg.run_query(&CUSTOMER_DEF, &query_not_empty).await.unwrap();

    // 3. bulk_create with empty items -> returns Ok(vec![])
    let created = pg.bulk_create(&CUSTOMER_DEF, None, Vec::new()).await.unwrap();
    assert!(created.is_empty());

    // 4. bulk_destroy with empty IDs -> returns Ok(())
    pg.bulk_destroy(&CUSTOMER_DEF, None, &[]).await.unwrap();
}

static NULLABLE_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::optional("owner_id", AttrType::Uuid),
    AttributeDef::optional("score", AttrType::Integer),
    AttributeDef::optional("active", AttrType::Boolean),
    AttributeDef::optional("settings", AttrType::Map),
];

static NULLABLE_DEF: ResourceDef = ResourceDef {
    name: "NullableSample",
    table: "nullable_samples",
    attributes: NULLABLE_ATTRS,
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

#[tokio::test]
async fn test_postgres_stores_null_in_typed_columns() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&NULLABLE_DEF]).await.unwrap();

    let id = Uuid::new_v4();
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    for name in ["owner_id", "score", "active", "settings"] {
        fields.insert(name.into(), Value::Null);
    }
    let created = pg.create(&NULLABLE_DEF, None, id, fields).await.unwrap();
    for name in ["owner_id", "score", "active", "settings"] {
        assert_eq!(created.get(name), Some(&Value::Null), "{name}");
    }

    let rows = pg
        .run_query(
            &NULLABLE_DEF,
            &CompiledQuery {
                filter: Some(Filter::and([
                    Filter::eq("id", id),
                    Filter::is_nil("owner_id"),
                    Filter::is_nil("score"),
                ])),
                ..CompiledQuery::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
}

static BONUS_CALCS: &[ash_core::CalculationDef] = &[ash_core::CalculationDef::with_arguments(
    "bonus",
    AttrType::Integer,
    ash_core::Expr::Add(
        &ash_core::Expr::Field("score"),
        &ash_core::Expr::Coalesce(&[&ash_core::Expr::Arg("extra"), &ash_core::Expr::LitInt(0)]),
    ),
    &[ash_core::ArgumentDef::new("extra", AttrType::Integer)],
)];

static BONUS_DEF: ResourceDef = ResourceDef {
    name: "BonusSample",
    table: "bonus_samples",
    calculations: BONUS_CALCS,
    ..NULLABLE_DEF
};

#[tokio::test]
async fn test_postgres_binds_missing_calculation_arguments_with_their_type() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&BONUS_DEF]).await.unwrap();
    let id = Uuid::new_v4();
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("score".into(), Value::Int(5));
    pg.create(&BONUS_DEF, None, id, fields).await.unwrap();

    // Without `extra`, COALESCE needs an integer NULL; a text NULL would not match 0.
    let query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(id))),
        calculations: vec!["bonus".into()],
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&BONUS_DEF, &query).await.unwrap();
    assert_eq!(rows[0].get("bonus"), Some(&Value::Int(5)));
}

static TENANT_NOTE_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("org", AttrType::String),
    AttributeDef::required("body", AttrType::String),
];

static TENANT_NOTE_DEF: ResourceDef = ResourceDef {
    name: "TenantNote",
    table: "tenant_notes",
    attributes: TENANT_NOTE_ATTRS,
    multitenancy: Some(ash_core::MultitenancyDef::attribute("org")),
    ..NULLABLE_DEF
};

#[tokio::test]
async fn test_postgres_attribute_tenancy_keeps_the_search_path_in_transactions() {
    let Some(admin) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    // The app's tables live outside `public`, as they do with one schema per deployment.
    let schema = format!("app_{}", Uuid::new_v4().simple());
    admin.execute_sql(&format!("CREATE SCHEMA \"{schema}\"")).await.unwrap();
    let base = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".to_string());
    let separator = if base.contains('?') { '&' } else { '?' };
    let pg = Postgres::connect(&format!("{base}{separator}options=-c%20search_path%3D{schema}"))
        .await
        .unwrap();
    pg.install(&[&TENANT_NOTE_DEF]).await.unwrap();

    let id = Uuid::new_v4();
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("org".into(), Value::String("acme".into()));
    fields.insert("body".into(), Value::String("hello".into()));
    pg.create(&TENANT_NOTE_DEF, None, id, fields).await.unwrap();

    // A tenant names a row filter here, not a schema, so it must not move the search_path.
    let query = CompiledQuery {
        filter: Some(Filter::eq("org", "acme")),
        tenant: Some("acme".into()),
        ..CompiledQuery::default()
    };
    let rows = pg
        .transaction(|tx| {
            let tx = tx.clone();
            async move {
                let first = tx.run_query(&TENANT_NOTE_DEF, &query).await?;
                let second = tx.run_query(&TENANT_NOTE_DEF, &query).await?;
                Ok((first.len(), second.len()))
            }
        })
        .await
        .unwrap();
    assert_eq!(rows, (1, 1));
}

static SUM_LINE_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("order_id", AttrType::Uuid),
    AttributeDef::required("amount", AttrType::Integer),
];

static SUM_LINE_DEF: ResourceDef = ResourceDef {
    name: "SumLine",
    table: "sum_lines",
    attributes: SUM_LINE_ATTRS,
    ..NULLABLE_DEF
};

static SUM_ORDER_RELS: &[ash_core::RelationshipDef] =
    &[ash_core::RelationshipDef::has_many("lines", || &SUM_LINE_DEF, "order_id")];

static SUM_ORDER_AGGS: &[ash_core::AggregateDef] =
    &[ash_core::AggregateDef::sum("total", "lines", "amount")];

static SUM_ORDER_DEF: ResourceDef = ResourceDef {
    name: "SumOrder",
    table: "sum_orders",
    attributes: &[AttributeDef::uuid_pk("id")],
    relationships: SUM_ORDER_RELS,
    aggregates: SUM_ORDER_AGGS,
    ..NULLABLE_DEF
};

#[tokio::test]
async fn test_postgres_sum_aggregates_read_as_integers() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&SUM_ORDER_DEF, &SUM_LINE_DEF]).await.unwrap();
    let order = Uuid::new_v4();
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(order));
    pg.create(&SUM_ORDER_DEF, None, order, fields).await.unwrap();
    for amount in [12, 30] {
        let id = Uuid::new_v4();
        let mut fields = FieldMap::new();
        fields.insert("id".into(), Value::Uuid(id));
        fields.insert("order_id".into(), Value::Uuid(order));
        fields.insert("amount".into(), Value::Int(amount));
        pg.create(&SUM_LINE_DEF, None, id, fields).await.unwrap();
    }

    // `SUM(bigint)` is `numeric` in Postgres.
    let query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(order))),
        aggregates: vec!["total".into()],
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&SUM_ORDER_DEF, &query).await.unwrap();
    assert_eq!(rows[0].get("total"), Some(&Value::Int(42)));
}

/// An aggregate is its subquery wherever a filter or sort refers to it, as AshPostgres
/// writes it, and a read selects only the attributes asked for.
#[tokio::test]
async fn test_postgres_filters_and_sorts_by_aggregates_and_selects_attributes() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&SUM_ORDER_DEF, &SUM_LINE_DEF]).await.unwrap();
    let mut orders = Vec::new();
    for amounts in [&[12, 30][..], &[5][..]] {
        let order = Uuid::new_v4();
        orders.push(order);
        pg.create(&SUM_ORDER_DEF, None, order, FieldMap::from([("id".into(), Value::Uuid(order))])).await.unwrap();
        for amount in amounts {
            let id = Uuid::new_v4();
            let fields = FieldMap::from([
                ("id".into(), Value::Uuid(id)),
                ("order_id".into(), Value::Uuid(order)),
                ("amount".into(), Value::Int(*amount)),
            ]);
            pg.create(&SUM_LINE_DEF, None, id, fields).await.unwrap();
        }
    }
    let ours = Filter::in_list("id", orders.iter().copied().map(Value::Uuid));

    let query = CompiledQuery {
        filter: Some(Filter::and([ours.clone(), Filter::gt("total", Value::Int(10))])),
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&SUM_ORDER_DEF, &query).await.unwrap();
    assert_eq!(rows.iter().map(|row| row.get("id")).collect::<Vec<_>>(), [Some(&Value::Uuid(orders[0]))]);

    let query = CompiledQuery {
        filter: Some(ours),
        sort: vec![ash_core::Sort { field: "total".into(), descending: false }],
        aggregates: vec!["total".into()],
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&SUM_ORDER_DEF, &query).await.unwrap();
    let totals: Vec<_> = rows.iter().map(|row| row.get("total").cloned()).collect();
    assert_eq!(totals, [Some(Value::Int(5)), Some(Value::Int(42))]);

    // Only the primary key and what's selected.
    let query = CompiledQuery {
        filter: Some(Filter::eq("order_id", Value::Uuid(orders[0]))),
        select: Some(vec!["amount".into()]),
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&SUM_LINE_DEF, &query).await.unwrap();
    assert_eq!(rows.len(), 2);
    for row in rows {
        let mut keys: Vec<_> = row.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["amount", "id"]);
    }
}

static OWNED_LINE_DEF: ResourceDef = ResourceDef {
    name: "OwnedLine",
    table: "owned_lines",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("order_id", AttrType::Uuid),
        AttributeDef::required("owner_id", AttrType::Uuid),
        AttributeDef::required("at", AttrType::UTC_DATETIME_USEC),
    ],
    actions: &[ash_core::ActionDef::read("read").primary()],
    // Each line is its owner's to read.
    policies: &[ash_core::PolicyDef::when(
        ash_core::PolicyWhen::ActionType(ash_core::ActionKind::Read),
        &[ash_core::PolicyEffect::AuthorizeIf(ash_core::Check::RelatesToActor { field: "owner_id" })],
    )],
    ..NULLABLE_DEF
};

static OWNED_ORDER_DEF: ResourceDef = ResourceDef {
    name: "OwnedOrder",
    table: "owned_orders",
    attributes: &[AttributeDef::uuid_pk("id")],
    relationships: &[ash_core::RelationshipDef::has_many("lines", || &OWNED_LINE_DEF, "order_id")],
    aggregates: &[
        ash_core::AggregateDef::count("line_count", "lines"),
        ash_core::AggregateDef::first("first_at", "lines", "at", AttrType::UTC_DATETIME_USEC),
    ],
    ..NULLABLE_DEF
};

/// An aggregate counts only the related rows its actor may read, as Ash authorizes an
/// aggregate's query by default; and a filter on an aggregate binds its value as the
/// aggregate's type.
#[tokio::test]
async fn test_postgres_aggregates_count_what_the_actor_reads() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&OWNED_ORDER_DEF, &OWNED_LINE_DEF]).await.unwrap();
    let order = Uuid::new_v4();
    pg.create(&OWNED_ORDER_DEF, None, order, FieldMap::from([("id".into(), Value::Uuid(order))])).await.unwrap();
    let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
    for owner in [mine, mine, theirs] {
        let id = Uuid::new_v4();
        let fields = FieldMap::from([
            ("id".into(), Value::Uuid(id)),
            ("order_id".into(), Value::Uuid(order)),
            ("owner_id".into(), Value::Uuid(owner)),
            ("at".into(), Value::from("2026-01-02T03:04:05.000000Z")),
        ]);
        pg.create(&OWNED_LINE_DEF, None, id, fields).await.unwrap();
    }
    let count_as = |actor: Option<ash_core::Actor>| CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(order))),
        aggregates: vec!["line_count".into()],
        actor,
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&OWNED_ORDER_DEF, &count_as(Some(ash_core::Actor::new(mine)))).await.unwrap();
    assert_eq!(rows[0].get("line_count"), Some(&Value::Int(2)));
    let rows = pg.run_query(&OWNED_ORDER_DEF, &count_as(None)).await.unwrap();
    assert_eq!(rows[0].get("line_count"), Some(&Value::Int(0)));

    let query = CompiledQuery {
        filter: Some(Filter::and([
            Filter::eq("id", Value::Uuid(order)),
            Filter::gt("first_at", Value::from("2026-01-01T00:00:00.000000Z")),
        ])),
        actor: Some(ash_core::Actor::new(mine)),
        ..CompiledQuery::default()
    };
    assert_eq!(pg.run_query(&OWNED_ORDER_DEF, &query).await.unwrap().len(), 1);
}

fn shout(fields: &FieldMap) -> ash_core::Result<Value> {
    Ok(fields.get("label").and_then(Value::as_str).map(|t| Value::String(t.to_uppercase())).unwrap_or(Value::Null))
}

static SHOUTING_DEF: ResourceDef = ResourceDef {
    name: "Shouting",
    table: "shoutings",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("label", AttrType::String)],
    calculations: &[ash_core::CalculationDef::new("shout", AttrType::String, ash_core::Expr::Custom(shout))],
    ..NULLABLE_DEF
};

/// A calculation only Rust can compute loads from Postgres too: computed from the record
/// once it's read, every attribute read for it, whatever the query selects.
#[tokio::test]
async fn test_postgres_computes_rust_only_calculations() {
    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&SHOUTING_DEF]).await.unwrap();
    let id = Uuid::new_v4();
    let fields = FieldMap::from([("id".into(), Value::Uuid(id)), ("label".into(), Value::from("quiet"))]);
    pg.create(&SHOUTING_DEF, None, id, fields).await.unwrap();
    let query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(id))),
        select: Some(Vec::new()),
        calculations: vec!["shout".into()],
        ..CompiledQuery::default()
    };
    let rows = pg.run_query(&SHOUTING_DEF, &query).await.unwrap();
    assert_eq!(rows[0].get("shout"), Some(&Value::from("QUIET")));
}

mod pg_shift {
    use ash_core::{UtcDateTime, UtcDateTimeUsec, resource};
    use uuid::Uuid;

    resource! {
        PgShift {
            table "pg_shifts";

            attributes {
                id: Uuid [pk];
                label: String;
                starts_at: UtcDateTime;
                logged_at: Option<UtcDateTimeUsec>;
            }

            actions {
                create create { primary; accept [label, starts_at, logged_at]; }
                read read { primary; }
            }
        }
    }
}

/// Postgres returns datetimes in the same UTC form, at the same precision, as memory
/// and SQLite store them, and compares filter values the same way.
#[tokio::test]
async fn test_postgres_datetimes_round_trip_in_utc_at_their_precision() {
    use ash_core::{Context, Resource, UtcDateTime, UtcDateTimeUsec};
    use pg_shift::PgShift;

    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&PgShift::DEF]).await.unwrap();
    let ctx = Context::new(pg);
    let run = Uuid::new_v4().to_string();
    let at = |raw: &str| UtcDateTime::parse(raw).unwrap();
    PgShift::create(&ctx)
        .label(format!("{run}-late"))
        .starts_at(at("2187-02-12T01:30:00+02:00"))
        .logged_at(UtcDateTimeUsec::parse("2187-02-12T08:00:00.5+00:00").unwrap())
        .await
        .unwrap();
    PgShift::create(&ctx)
        .label(format!("{run}-early"))
        .starts_at(at("2187-02-11T23:45:00.900Z"))
        .await
        .unwrap();

    let ours = Filter::starts_with("label", run.clone());
    let shifts = PgShift::query(&ctx)
        .filter(ours.clone())
        .sort(PgShift::starts_at)
        .all()
        .await
        .unwrap();
    assert_eq!(shifts[0].label, format!("{run}-late"));
    assert_eq!(shifts[0].starts_at.as_str(), "2187-02-11T23:30:00Z");
    assert_eq!(shifts[1].starts_at.as_str(), "2187-02-11T23:45:00Z");
    assert_eq!(
        shifts[0].logged_at.as_ref().map(UtcDateTimeUsec::as_str),
        Some("2187-02-12T08:00:00.500000Z")
    );

    // A fraction below the attribute's precision is dropped from the filter value too.
    let early = PgShift::query(&ctx)
        .filter(Filter::and([
            ours,
            Filter::eq("starts_at", Value::String("2187-02-11T23:45:00.900Z".into())),
        ]))
        .all()
        .await
        .unwrap();
    assert_eq!(early.len(), 1);
}

/// Context multitenancy on Postgres: each tenant's rows live in its own schema, and every
/// statement names the tenant's table, as AshPostgres's schema prefixes do, inside a
/// transaction or not.
mod context_tenancy {
    use ash_core::{Context, DataLayer, Error, Filter, Resource, TransactionSupport};

    pub use booking::Booking;
    pub use port::Port;

    pub mod port {
        use super::booking::Booking;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            /// Shared by every tenant.
            Port {
                table "pg_ctx_ports";

                attributes {
                    id: Uuid [pk];
                    code: String;
                }

                relationships {
                    has_many bookings: Booking [fk: port_id];
                }

                aggregates {
                    booking_count: Option<i64> = count(bookings);
                    booked_tonnes: Option<i64> = sum(bookings, tonnes);
                }

                actions {
                    create create { primary; accept [code]; }
                    read read { primary; }
                }
            }
        }
    }

    pub mod booking {
        use super::port::Port;
        use ash_core::resource;
        use uuid::Uuid;

        resource! {
            /// Each tenant's bookings live apart from the others'.
            Booking {
                table "pg_ctx_bookings";

                multitenancy {
                    strategy: context;
                }

                attributes {
                    id: Uuid [pk];
                    port_id: Uuid;
                    tonnes: i64;
                }

                relationships {
                    belongs_to port: Port [fk: port_id];
                }

                actions {
                    create create { primary; accept [port_id, tonnes]; }
                    read read { primary; }
                    update weigh { primary; accept [tonnes]; }
                    destroy cancel { primary; }
                }
            }
        }
    }

    /// Every read and write of a context-tenant resource stays in its tenant: direct reads,
    /// writes by id, aggregates and filters through a relationship, loads, and writes in a
    /// transaction.
    pub async fn tenants_stay_apart<D: DataLayer + TransactionSupport + Clone>(ctx: Context<D>, acme: &str, globex: &str) {
        let acme = ctx.with_tenant(acme);
        let globex = ctx.with_tenant(globex);
        let leo = Port::create(&ctx).code("LEO").await.unwrap();
        let ours = Booking::create(&acme).port_id(leo.id).tonnes(10).await.unwrap();
        Booking::create(&acme).port_id(leo.id).tonnes(30).await.unwrap();
        let theirs = Booking::create(&globex).port_id(leo.id).tonnes(5).await.unwrap();

        let tonnes = |ctx: Context<D>| async move {
            let mut tonnes: Vec<i64> = Booking::query(&ctx).all().await.unwrap().iter().map(|b| b.tonnes).collect();
            tonnes.sort();
            tonnes
        };
        assert_eq!(tonnes(acme.clone()).await, [10, 30]);
        assert_eq!(tonnes(globex.clone()).await, [5]);

        // Knowing another tenant's id reaches nothing.
        assert!(matches!(Booking::get(&globex, ours.id).await, Err(Error::NotFound)));
        let err = ash_core::update_dynamic(
            &globex,
            &Booking::DEF,
            Booking::DEF.action("weigh").unwrap(),
            ours.id,
            [("tonnes".to_string(), ash_core::Value::Int(99))].into_iter().collect(),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, Error::NotFound), "{err:?}");
        theirs.cancel_on(&globex).await.unwrap();
        assert_eq!(tonnes(globex.clone()).await, Vec::<i64>::new());
        assert_eq!(tonnes(acme.clone()).await, [10, 30]);

        // Reads through a relationship see the tenant's rows only.
        let port = Port::query(&acme)
            .filter(Filter::eq("id", leo.id))
            .load_aggregate(Port::booking_count)
            .load_aggregate(Port::booked_tonnes)
            .load_rel(Port::bookings)
            .one()
            .await
            .unwrap();
        assert_eq!((port.booking_count, port.booked_tonnes), (Some(2), Some(40)));
        assert_eq!(port.bookings.loaded().unwrap().len(), 2);
        let heavy = Port::query(&globex)
            .filter(Filter::eq("id", leo.id) & Filter::related("bookings", Filter::gt("tonnes", 20)))
            .all()
            .await
            .unwrap();
        assert!(heavy.is_empty());

        // Writes in a transaction land in the tenant too.
        let in_tx = acme
            .transaction(|tx| async move { Booking::create(&tx).port_id(leo.id).tonnes(1).await })
            .await
            .unwrap();
        assert!(Booking::get(&acme, in_tx.id).await.is_ok());
        assert!(matches!(Booking::get(&globex, in_tx.id).await, Err(Error::NotFound)));

        // A tenant-scoped resource needs a tenant.
        assert!(matches!(
            Booking::query(&ctx).all().await,
            Err(Error::TenantRequired { .. })
        ));
    }


    #[tokio::test]
    async fn postgres_keeps_tenants_apart_in_schemas() {
        let Some(pg) = super::get_test_postgres().await else {
            eprintln!("PostgreSQL not reachable; skipping test");
            return;
        };
        pg.install(&[&Port::DEF, &Booking::DEF]).await.unwrap();
        let run = uuid::Uuid::new_v4().simple().to_string();
        let (acme, globex) = (format!("acme_{run}"), format!("globex_{run}"));
        for tenant in [&acme, &globex] {
            pg.install_tenant(tenant, &[&Port::DEF, &Booking::DEF]).await.unwrap();
        }
        tenants_stay_apart(Context::new(pg), &acme, &globex).await;
    }
}

mod pg_cycle {
    pub mod car {
        use ash_core::resource;
        use uuid::Uuid;

        use super::driver::PgDriver;

        resource! {
            PgCar {
                table "pg_cars";

                attributes {
                    id: Uuid [pk];
                    plate: String;
                    driver_id: Option<Uuid>;
                }

                relationships {
                    belongs_to driver: PgDriver [fk: driver_id];
                }

                actions {
                    create create { primary; accept [plate, driver_id]; }
                    read read { primary; }
                }
            }
        }
    }

    pub mod driver {
        use ash_core::resource;
        use uuid::Uuid;

        use super::car::PgCar;

        resource! {
            PgDriver {
                table "pg_drivers";

                attributes {
                    id: Uuid [pk];
                    name: String;
                    car_id: Option<Uuid>;
                }

                relationships {
                    belongs_to car: PgCar [fk: car_id];
                }

                actions {
                    create create { primary; accept [name, car_id]; }
                    read read { primary; }
                    update assign { accept [car_id]; }
                }
            }
        }
    }
}

/// Two tables that refer to each other install: neither can be created first with its
/// foreign key inline, so the keys are added once both exist. Installing again changes
/// nothing, and the keys hold.
#[tokio::test]
async fn test_postgres_installs_tables_that_refer_to_each_other() {
    use ash_core::{Context, Resource};
    use pg_cycle::{car::PgCar, driver::PgDriver};

    let Some(admin) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    let schema = format!("cycle_{}", Uuid::new_v4().simple());
    admin.execute_sql(&format!("CREATE SCHEMA \"{schema}\"")).await.unwrap();
    let base = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".to_string());
    let separator = if base.contains('?') { '&' } else { '?' };
    let pg = Postgres::connect(&format!("{base}{separator}options=-c%20search_path%3D{schema}"))
        .await
        .unwrap();
    pg.install(&[&PgCar::DEF, &PgDriver::DEF]).await.unwrap();
    pg.install(&[&PgCar::DEF, &PgDriver::DEF]).await.unwrap();

    let ctx = Context::new(pg);
    let driver = PgDriver::create(&ctx).name("Ada".to_string()).await.unwrap();
    let car = PgCar::create(&ctx)
        .plate("CYBR-1".to_string())
        .driver_id(Some(driver.id))
        .await
        .unwrap();
    let driver = driver.assign_on(&ctx).car_id(Some(car.id)).await.unwrap();
    assert_eq!(driver.car_id, Some(car.id));

    // Both keys are enforced.
    let missing = Some(Uuid::new_v4());
    assert!(PgCar::create(&ctx).plate("GHOST".to_string()).driver_id(missing).await.is_err());
    assert!(PgDriver::create(&ctx).name("Nobody".to_string()).car_id(missing).await.is_err());
}

mod pg_fleet {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        PgVehicle {
            table "pg_vehicles";

            attributes {
                id: Uuid [pk];
                call_sign: String;
                lng: f64;
                speed_kph: i64;
                status: String;
            }

            actions {
                create create { primary; accept [call_sign, lng, speed_kph, status]; }
                read read { primary; }
                update report { accept [lng, speed_kph, status]; }
                destroy destroy { primary; }
            }
        }
    }
}

/// A bulk update writes each row's own changes, grouping rows that change the same
/// columns into one statement, and a row that's gone fails alone.
#[tokio::test]
async fn test_postgres_bulk_update_writes_each_rows_changes() {
    use ash_core::{BulkUpdateOptions, Context, DataLayer, FieldMap, Resource};
    use pg_fleet::PgVehicle;

    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&PgVehicle::DEF]).await.unwrap();
    let ctx = Context::new(pg);
    let run = Uuid::new_v4().simple().to_string();
    let mut fleet = Vec::new();
    for i in 0..5 {
        fleet.push(
            PgVehicle::create(&ctx)
                .call_sign(format!("{run}-{i}"))
                .lng(-97.7)
                .speed_kph(0)
                .status("available".to_string())
                .await
                .unwrap(),
        );
    }
    // One is gone before the batch lands.
    ctx.data.destroy(&PgVehicle::DEF, None, fleet[4].id).await.unwrap();

    let updates = fleet.iter().cloned().enumerate().map(|(i, vehicle)| {
        let mut input = FieldMap::new();
        input.insert("lng".into(), Value::from(-97.7 + i as f64 / 100.0));
        input.insert("speed_kph".into(), Value::from(10 * i as i64));
        // Two of them also change status: a second group of columns.
        if i % 2 == 1 {
            input.insert("status".into(), Value::from("on_trip"));
        }
        (vehicle, input)
    });
    let result = PgVehicle::bulk_update_with_opts(
        &ctx,
        "report",
        updates,
        BulkUpdateOptions::new().stop_on_error(false),
    )
    .await
    .unwrap();
    assert_eq!((result.count, result.error_count), (4, 1), "{:?}", result.errors);

    let mut stored: Vec<(String, f64, i64, String)> = PgVehicle::query(&ctx)
        .load()
        .await
        .unwrap()
        .into_iter()
        .filter(|v| v.call_sign.starts_with(&run))
        .map(|v| (v.call_sign, v.lng, v.speed_kph, v.status))
        .collect();
    stored.sort_by(|a, b| a.0.cmp(&b.0));
    let expected: Vec<(String, f64, i64, String)> = (0..4)
        .map(|i| {
            let status = if i % 2 == 1 { "on_trip" } else { "available" };
            (format!("{run}-{i}"), -97.7 + i as f64 / 100.0, 10 * i as i64, status.to_string())
        })
        .collect();
    assert_eq!(stored, expected);
}

/// A counted page reads its count alongside the page; in a transaction, whose one
/// connection takes them in turn, it counts and pages as it does outside one.
#[tokio::test]
async fn test_postgres_counted_page_in_and_out_of_a_transaction() {
    use ash_core::{Context, Resource};
    use pg_fleet::PgVehicle;

    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&PgVehicle::DEF]).await.unwrap();
    let ctx = Context::new(pg);
    let run = Uuid::new_v4().simple().to_string();
    for i in 0..5 {
        PgVehicle::create(&ctx)
            .call_sign(format!("{run}-{i}"))
            .lng(-97.7)
            .speed_kph(0)
            .status(run.clone())
            .await
            .unwrap();
    }
    let page = |ctx: Context<Postgres>, run: String| async move {
        let page = PgVehicle::query(&ctx)
            .filter(Filter::eq("status", run))
            .page_offset(2, 0, true)
            .await?;
        Ok::<_, ash_core::Error>((page.results.len(), page.total_count))
    };

    assert_eq!(page(ctx.clone(), run.clone()).await.unwrap(), (2, Some(5)));
    let in_transaction = ctx.transaction(|tx| page(tx, run.clone())).await.unwrap();
    assert_eq!(in_transaction, (2, Some(5)));
}

/// Ash's like/ilike reach Postgres as LIKE and ILIKE, wildcards and escapes intact.
#[tokio::test]
async fn test_postgres_like_and_ilike() {
    use ash_core::{Context, Filter, Resource};
    use pg_fleet::PgVehicle;

    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&PgVehicle::DEF]).await.unwrap();
    let ctx = Context::new(pg);
    let run = Uuid::new_v4().simple().to_string();
    for sign in ["LIKE-A_1", "LIKE-B2", "like-c3"] {
        PgVehicle::create(&ctx)
            .call_sign(format!("{run}{sign}"))
            .lng(0.0)
            .speed_kph(0)
            .status("available".to_string())
            .await
            .unwrap();
    }
    let signs = |filter: Filter| {
        let ctx = ctx.clone();
        let run = run.clone();
        async move {
            let mut signs: Vec<String> = PgVehicle::query(&ctx)
                .filter(Filter::And(vec![Filter::starts_with("call_sign", run.clone()), filter]))
                .load()
                .await
                .unwrap()
                .into_iter()
                .map(|v| v.call_sign.trim_start_matches(&run).to_string())
                .collect();
            signs.sort();
            signs
        }
    };
    assert_eq!(signs(Filter::like("call_sign", "%LIKE-%")).await, ["LIKE-A_1", "LIKE-B2"]);
    assert_eq!(signs(Filter::ilike("call_sign", "%like-%")).await, ["LIKE-A_1", "LIKE-B2", "like-c3"]);
    assert_eq!(signs(Filter::like("call_sign", "%A\\_1")).await, ["LIKE-A_1"]);
    assert_eq!(signs(Filter::like("call_sign", "%LIKE-__")).await, ["LIKE-B2"]);
}

/// Counts run in the database, as `COUNT(*)`: of a filter, and of a page.
#[tokio::test]
async fn test_postgres_counts_in_the_database() {
    use ash_core::{CompiledQuery, Context, DataLayer, Filter, Resource};
    use pg_fleet::PgVehicle;

    let Some(pg) = get_test_postgres().await else {
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&PgVehicle::DEF]).await.unwrap();
    let ctx = Context::new(pg.clone());
    let run = Uuid::new_v4().simple().to_string();
    for (i, status) in ["available", "available", "charging"].into_iter().enumerate() {
        PgVehicle::create(&ctx)
            .call_sign(format!("{run}-{i}"))
            .lng(0.0)
            .speed_kph(0)
            .status(status.to_string())
            .await
            .unwrap();
    }
    let ours = Filter::starts_with("call_sign", run.clone());
    let count = |filter: Filter| PgVehicle::query(&ctx).filter(filter).count();
    assert_eq!(count(ours.clone()).await.unwrap(), 3);
    assert_eq!(
        count(Filter::And(vec![ours.clone(), Filter::eq("status", "available")])).await.unwrap(),
        2
    );
    let page = CompiledQuery {
        filter: Some(ours),
        limit: Some(2),
        offset: Some(2),
        ..CompiledQuery::default()
    };
    assert_eq!(pg.count(&PgVehicle::DEF, &page).await.unwrap(), 1);
}
