use ash_core::{
    ActionDef, AttrType, AttributeDef, CalculationDef, CompiledQuery, Expr, Filter, IdentityDef,
    ResourceDef, Sort, Value,
};
use ash_sql::{CompiledSql, PostgresDialect, QueryCompiler, SqliteDialect};
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("subject", AttrType::String),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["open", "closed"],
        },
    ),
    AttributeDef::optional("representative_id", AttrType::Uuid),
    AttributeDef::optional("priority", AttrType::Integer),
];

static TICKET_CALCS: &[CalculationDef] = &[CalculationDef::new(
    "subject_length",
    AttrType::Integer,
    Expr::StringLength("subject"),
)];

static TICKET_IDENTS: &[IdentityDef] = &[IdentityDef::new("unique_subject", &["subject"])];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: &[],
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: TICKET_CALCS,
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: TICKET_IDENTS,
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

#[test]
fn test_sqlite_query_compilation() {
    let dialect = SqliteDialect;
    let mut compiler = QueryCompiler::new(&dialect);

    let query = CompiledQuery {
        filter: Some(Filter::and([
            Filter::eq("status", "open"),
            Filter::or([
                Filter::is_nil("representative_id"),
                Filter::gt("priority", 2_i64),
            ]),
        ])),
        sort: vec![
            Sort {
                field: "priority".into(),
                descending: false,
            },
            Sort {
                field: "subject".into(),
                descending: true,
            },
        ],
        limit: Some(10),
        offset: Some(20),
        calculations: vec!["subject_length".into()],
        ..CompiledQuery::default()
    };

    let CompiledSql { sql, params } = compiler.compile_select(&TICKET_DEF, &query).unwrap();

    assert!(sql.contains("SELECT \"id\", \"subject\", \"status\", \"representative_id\", \"priority\", length(\"subject\") AS \"subject_length\" FROM \"tickets\""));
    assert!(sql.contains("WHERE (\"status\" = ? AND (\"representative_id\" IS NULL OR \"priority\" > ?))"));
    assert!(sql.contains("ORDER BY \"priority\" ASC, \"subject\" DESC"));
    assert!(sql.contains("LIMIT ? OFFSET ?"));

    // Check placeholder in SQLite is ?
    assert_eq!(params.len(), 4);
    assert_eq!(params[0].value, Value::String("open".into()));
    assert_eq!(params[1].value, Value::Int(2));
    assert_eq!(params[2].value, Value::Int(10));
    assert_eq!(params[3].value, Value::Int(20));
}

#[test]
fn test_postgres_query_compilation() {
    let dialect = PostgresDialect;
    let mut compiler = QueryCompiler::new(&dialect);

    let query = CompiledQuery {
        filter: Some(Filter::and([
            Filter::eq("status", "open"),
            Filter::gt("subject_length", 5_i64),
        ])),
        limit: Some(5),
        ..CompiledQuery::default()
    };

    let CompiledSql { sql, params } = compiler.compile_select(&TICKET_DEF, &query).unwrap();

    assert!(sql.contains("WHERE (\"status\" = $1 AND length(\"subject\") > $2)"));
    assert!(sql.contains("LIMIT $3"));
    assert_eq!(params.len(), 3);
}

#[test]
fn test_postgres_insert_update_upsert_returning() {
    let dialect = PostgresDialect;

    // 1. Insert
    let mut compiler = QueryCompiler::new(&dialect);
    let mut fields = ash_core::FieldMap::new();
    let id = Uuid::new_v4();
    fields.insert("id".into(), Value::Uuid(id));
    fields.insert("subject".into(), Value::String("Server Down".into()));
    fields.insert("status".into(), Value::String("open".into()));

    let compiled = compiler.compile_insert(&TICKET_DEF, &fields).unwrap();
    assert!(compiled.sql.contains("INSERT INTO \"tickets\""));
    assert!(compiled.sql.contains("RETURNING *"));
    assert_eq!(compiled.params.len(), 3);

    // 2. Update
    let mut compiler2 = QueryCompiler::new(&dialect);
    let mut update_fields = ash_core::FieldMap::new();
    update_fields.insert("status".into(), Value::String("closed".into()));

    let compiled_up = compiler2.compile_update(&TICKET_DEF, id, &update_fields).unwrap();
    assert!(compiled_up.sql.contains("UPDATE \"tickets\" SET \"status\" = $1 WHERE \"id\" = $2 RETURNING *"));

    // 3. Upsert
    let mut compiler3 = QueryCompiler::new(&dialect);
    let compiled_upsert = compiler3
        .compile_upsert(&TICKET_DEF, &fields, &TICKET_IDENTS[0], &["status".to_string()])
        .unwrap();
    assert!(compiled_upsert.sql.contains("ON CONFLICT (\"subject\") DO UPDATE SET \"status\" = EXCLUDED.\"status\" RETURNING *"));
}

#[test]
fn test_create_table_ddl_compilation() {
    let sqlite_ddl = QueryCompiler::new(&SqliteDialect).compile_create_table(&TICKET_DEF).unwrap();
    assert!(sqlite_ddl.contains("CREATE TABLE IF NOT EXISTS \"tickets\""));
    assert!(sqlite_ddl.contains("\"id\" TEXT PRIMARY KEY"));
    assert!(sqlite_ddl.contains("\"subject\" TEXT NOT NULL"));
    assert!(sqlite_ddl.contains("\"priority\" INTEGER"));

    let pg_ddl = QueryCompiler::new(&PostgresDialect).compile_create_table(&TICKET_DEF).unwrap();
    assert!(pg_ddl.contains("CREATE TABLE IF NOT EXISTS \"tickets\""));
    assert!(pg_ddl.contains("\"id\" UUID PRIMARY KEY"));
    assert!(pg_ddl.contains("\"subject\" TEXT NOT NULL"));
    assert!(pg_ddl.contains("\"priority\" BIGINT"));
}
