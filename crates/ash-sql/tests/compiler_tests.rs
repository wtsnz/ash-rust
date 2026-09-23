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
    indexes: &[],
    checks: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
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
    assert!(sql.contains(
        "WHERE (\"status\" = ? AND (\"representative_id\" IS NULL OR \"priority\" > ?))"
    ));
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

    let compiled_up = compiler2
        .compile_update(&TICKET_DEF, id, &update_fields)
        .unwrap();
    assert!(compiled_up
        .sql
        .contains("UPDATE \"tickets\" SET \"status\" = $1 WHERE \"id\" = $2 RETURNING *"));

    // 3. Upsert
    let mut compiler3 = QueryCompiler::new(&dialect);
    let compiled_upsert = compiler3
        .compile_upsert(
            &TICKET_DEF,
            &fields,
            &TICKET_IDENTS[0],
            &["status".to_string()],
        )
        .unwrap();
    assert!(compiled_upsert.sql.contains(
        "ON CONFLICT (\"subject\") DO UPDATE SET \"status\" = EXCLUDED.\"status\" RETURNING *"
    ));
}

#[test]
fn test_create_table_ddl_compilation() {
    let sqlite_ddl = QueryCompiler::new(&SqliteDialect)
        .compile_create_table(&TICKET_DEF)
        .unwrap();
    assert!(sqlite_ddl.contains("CREATE TABLE IF NOT EXISTS \"tickets\""));
    assert!(sqlite_ddl.contains("\"id\" TEXT PRIMARY KEY"));
    assert!(sqlite_ddl.contains("\"subject\" TEXT NOT NULL"));
    assert!(sqlite_ddl.contains("\"priority\" INTEGER"));

    let pg_ddl = QueryCompiler::new(&PostgresDialect)
        .compile_create_table(&TICKET_DEF)
        .unwrap();
    assert!(pg_ddl.contains("CREATE TABLE IF NOT EXISTS \"tickets\""));
    assert!(pg_ddl.contains("\"id\" UUID PRIMARY KEY"));
    assert!(pg_ddl.contains("\"subject\" TEXT NOT NULL"));
    assert!(pg_ddl.contains("\"priority\" BIGINT"));
}

#[test]
fn test_in_list_compilation_sqlite_json_each_vs_postgres_any() {
    let ids = vec![
        Value::Uuid(Uuid::new_v4()),
        Value::Uuid(Uuid::new_v4()),
        Value::Uuid(Uuid::new_v4()),
    ];
    let query = CompiledQuery {
        filter: Some(Filter::in_list("id", ids)),
        ..CompiledQuery::default()
    };

    // 1. SQLite dialect uses json_each with exactly 1 parameter
    let mut sqlite_compiler = QueryCompiler::new(&SqliteDialect);
    let sqlite_compiled = sqlite_compiler.compile_select(&TICKET_DEF, &query).unwrap();
    assert!(
        sqlite_compiled
            .sql
            .contains("\"id\" IN (SELECT value FROM json_each(?))"),
        "SQLite must compile IN query to json_each, got: {}",
        sqlite_compiled.sql
    );
    assert_eq!(
        sqlite_compiled.params.len(),
        1,
        "SQLite must bind array as a single parameter"
    );
    assert!(sqlite_compiled.params[0].is_list);

    // 2. Postgres dialect uses = ANY($1) with exactly 1 parameter
    let mut pg_compiler = QueryCompiler::new(&PostgresDialect);
    let pg_compiled = pg_compiler.compile_select(&TICKET_DEF, &query).unwrap();
    assert!(
        pg_compiled.sql.contains("\"id\" = ANY($1)"),
        "Postgres must compile IN query to = ANY($1), got: {}",
        pg_compiled.sql
    );
    assert_eq!(
        pg_compiled.params.len(),
        1,
        "Postgres must bind array as a single parameter"
    );
    assert!(pg_compiled.params[0].is_list);
}

static CATEGORY_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::optional("parent_id", AttrType::Uuid),
];

static CATEGORY_RELS: &[ash_core::RelationshipDef] = &[ash_core::RelationshipDef::has_many(
    "subcategories",
    || &CATEGORY_DEF,
    "parent_id",
)];

static CATEGORY_AGGS: &[ash_core::AggregateDef] = &[ash_core::AggregateDef::count(
    "subcategories_count",
    "subcategories",
)];

static CATEGORY_DEF: ResourceDef = ResourceDef {
    name: "Category",
    table: "categories",
    attributes: CATEGORY_ATTRS,
    relationships: CATEGORY_RELS,
    actions: &[ActionDef::read("read").primary()],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: CATEGORY_AGGS,
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[test]
fn test_self_referential_aggregate_subquery_aliasing() {
    let query = CompiledQuery {
        aggregates: vec!["subcategories_count".into()],
        ..CompiledQuery::default()
    };

    let mut compiler = QueryCompiler::new(&SqliteDialect);
    let compiled = compiler.compile_select(&CATEGORY_DEF, &query).unwrap();

    // Verify subquery table is aliased so outer "categories"."id" is not shadowed by inner table!
    assert!(
        compiled
            .sql
            .contains("FROM \"categories\" AS \"_ash_sub_subcategories_count\""),
        "Inner table must be aliased to prevent self-referential shadowing, got: {}",
        compiled.sql
    );
    assert!(
        compiled
            .sql
            .contains("\"_ash_sub_subcategories_count\".\"parent_id\" = \"categories\".\"id\""),
        "Inner alias must join against outer table, got: {}",
        compiled.sql
    );
}

#[test]
fn test_keyset_cursor_compilation() {
    let dialect = SqliteDialect;
    let mut compiler = QueryCompiler::new(&dialect);

    let cursor = ash_core::KeysetCursor {
        id: Uuid::nil(),
        values: vec![
            ("priority".to_string(), Value::Int(3)),
            ("subject".to_string(), Value::String("Alpha".to_string())),
        ],
    };

    let sorts = vec![
        Sort {
            field: "priority".to_string(),
            descending: true,
        },
        Sort {
            field: "subject".to_string(),
            descending: false,
        },
    ];

    let cursor_sql = compiler
        .compile_keyset_cursor(&TICKET_DEF, &cursor, &sorts)
        .unwrap();
    // Expected format: ("priority" < ? OR ("priority" = ? AND "subject" > ?) OR ("priority" = ? AND "subject" = ? AND "id" > ?))
    assert!(cursor_sql.contains("\"priority\" < ?"));
    assert!(cursor_sql.contains("\"priority\" = ? AND \"subject\" > ?"));
    assert!(cursor_sql.contains("\"priority\" = ? AND \"subject\" = ? AND \"id\" > ?"));

    // Also test compile_select_with_cursor appends the primary key tie-breaker to ORDER BY
    let mut select_compiler = QueryCompiler::new(&dialect);
    let select_query = CompiledQuery {
        sort: sorts,
        ..CompiledQuery::default()
    };
    let compiled_select = select_compiler
        .compile_select_with_cursor(&TICKET_DEF, &select_query, Some(&cursor))
        .unwrap();
    assert!(
        compiled_select
            .sql
            .contains("ORDER BY \"priority\" DESC, \"subject\" ASC, \"id\" ASC"),
        "Cursor queries must order deterministically with PK tie-breaker, got: {}",
        compiled_select.sql
    );
}

#[test]
fn test_empty_in_and_not_in_compilation() {
    let dialect = SqliteDialect;

    // Filter::in_list with empty vec
    let query_empty = CompiledQuery {
        filter: Some(Filter::in_list("id", Vec::<Value>::new())),
        ..CompiledQuery::default()
    };
    let mut compiler = QueryCompiler::new(&dialect);
    let compiled = compiler.compile_select(&TICKET_DEF, &query_empty).unwrap();
    assert!(
        compiled.sql.contains("WHERE 0=1"),
        "Empty IN list must compile to 0=1, got: {}",
        compiled.sql
    );

    // Filter::not(Filter::in_list) with empty vec
    let query_not_empty = CompiledQuery {
        filter: Some(!Filter::in_list("id", Vec::<Value>::new())),
        ..CompiledQuery::default()
    };
    let mut compiler2 = QueryCompiler::new(&dialect);
    let compiled2 = compiler2
        .compile_select(&TICKET_DEF, &query_not_empty)
        .unwrap();
    assert!(
        compiled2.sql.contains("WHERE NOT (0=1)"),
        "Negated empty IN must compile to NOT (0=1), got: {}",
        compiled2.sql
    );

    // compile_bulk_delete with empty IDs
    let mut compiler3 = QueryCompiler::new(&dialect);
    let compiled_del = compiler3.compile_bulk_delete(&TICKET_DEF, &[]).unwrap();
    assert!(
        compiled_del.sql.contains("WHERE 0=1"),
        "Bulk delete with 0 IDs must emit 0=1, got: {}",
        compiled_del.sql
    );
}

#[test]
fn test_complex_expressions_compilation() {
    let dialect = PostgresDialect;
    let mut compiler = QueryCompiler::new(&dialect);

    static COND: Expr = Expr::Gt(&Expr::StringLength("subject"), &Expr::LitInt(10));
    static THEN: Expr = Expr::Upper(&Expr::Field("subject"));
    static ELSE: Expr = Expr::LitString("short");

    let expr = Expr::IfElse {
        cond: &COND,
        then_expr: &THEN,
        else_expr: &ELSE,
    };

    let compiled_expr = compiler.compile_expr(&TICKET_DEF, &expr).unwrap();
    assert!(
        compiled_expr
            .contains("CASE WHEN (length(\"subject\") > 10) THEN upper(\"subject\") ELSE $1 END"),
        "Got: {}",
        compiled_expr
    );
}
