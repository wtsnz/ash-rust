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
            name: None,
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
    statements: &[],
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
                guard: None,
            },
            Sort {
                field: "subject".into(),
                descending: true,
                guard: None,
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
    statements: &[],
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
            .contains("FROM \"categories\" AS \"__ash_agg_subcategories\""),
        "Inner table must be aliased to prevent self-referential shadowing, got: {}",
        compiled.sql
    );
    assert!(
        compiled
            .sql
            .contains("\"__ash_aggs_subcategories\".\"__ash_key_0\" = \"__ash_s\".\"id\""),
        "Grouped counts must join on the outer record's key, got: {}",
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
            guard: None,
        },
        Sort {
            field: "subject".to_string(),
            descending: false,
            guard: None,
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

#[test]
fn test_text_filters_escape_wildcards_per_dialect() {
    let query = CompiledQuery {
        filter: Some(Filter::contains("subject", r"50%_off*[x]?\")),
        ..CompiledQuery::default()
    };

    let mut sqlite_compiler = QueryCompiler::new(&SqliteDialect);
    let sqlite = sqlite_compiler.compile_select(&TICKET_DEF, &query).unwrap();
    assert!(sqlite.sql.contains("\"subject\" GLOB ?"), "got: {}", sqlite.sql);
    assert_eq!(
        sqlite.params[0].value,
        Value::String(r"*50%_off[*][[]x][?]\*".into())
    );

    let mut pg_compiler = QueryCompiler::new(&PostgresDialect);
    let pg = pg_compiler.compile_select(&TICKET_DEF, &query).unwrap();
    assert!(pg.sql.contains("\"subject\" LIKE $1"), "got: {}", pg.sql);
    assert_eq!(
        pg.params[0].value,
        Value::String(r"%50\%\_off*[x]?\\%".into())
    );

    let starts = CompiledQuery {
        filter: Some(Filter::starts_with("status", "op")),
        ..CompiledQuery::default()
    };
    let mut pg_compiler = QueryCompiler::new(&PostgresDialect);
    let pg = pg_compiler.compile_select(&TICKET_DEF, &starts).unwrap();
    assert_eq!(pg.params[0].value, Value::String("op%".into()));

    let ends = CompiledQuery {
        filter: Some(Filter::ends_with("subject", "fire")),
        ..CompiledQuery::default()
    };
    let mut sqlite_compiler = QueryCompiler::new(&SqliteDialect);
    let sqlite = sqlite_compiler.compile_select(&TICKET_DEF, &ends).unwrap();
    assert_eq!(sqlite.params[0].value, Value::String("*fire".into()));
}

#[test]
fn test_text_filters_reject_non_text_fields() {
    for field in ["priority", "subject_length"] {
        let query = CompiledQuery {
            filter: Some(Filter::contains(field, "1")),
            ..CompiledQuery::default()
        };
        let mut compiler = QueryCompiler::new(&PostgresDialect);
        let err = compiler.compile_select(&TICKET_DEF, &query).unwrap_err();
        assert!(
            err.to_string().contains("text filters need a string field"),
            "{field}: {err}"
        );
    }
}

fn live_folders() -> Filter {
    Filter::and([
        Filter::is_nil("archived_at"),
        Filter::ne("name", "hidden"),
    ])
}

static FOLDER_PREPS: &[ash_core::PreparationDef] = &[ash_core::PreparationDef::Filter(live_folders)];

static FOLDER_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::optional("parent_id", AttrType::Uuid),
    AttributeDef::optional("archived_at", AttrType::UTC_DATETIME_USEC),
];

static FOLDER_RELS: &[ash_core::RelationshipDef] = &[ash_core::RelationshipDef::has_many(
    "children",
    || &FOLDER_DEF,
    "parent_id",
)];

static FOLDER_AGGS: &[ash_core::AggregateDef] =
    &[ash_core::AggregateDef::count("child_count", "children")];

static FOLDER_DEF: ResourceDef = ResourceDef {
    name: "Folder",
    table: "folders",
    attributes: FOLDER_ATTRS,
    relationships: FOLDER_RELS,
    actions: &[ActionDef::read("read").primary().preparations(FOLDER_PREPS)],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: FOLDER_AGGS,
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[test]
fn test_aggregates_and_related_filters_apply_the_destination_read_filter() {
    let query = CompiledQuery {
        filter: Some(Filter::and([
            Filter::eq("name", "root"),
            Filter::related("children", Filter::eq("name", "docs")),
        ])),
        aggregates: vec!["child_count".into()],
        ..CompiledQuery::default()
    };
    for (dialect, sql, params) in [
        {
            let mut compiler = QueryCompiler::new(&PostgresDialect);
            let compiled = compiler.compile_select(&FOLDER_DEF, &query).unwrap();
            ("postgres", compiled.sql, compiled.params)
        },
        {
            let mut compiler = QueryCompiler::new(&SqliteDialect);
            let compiled = compiler.compile_select(&FOLDER_DEF, &query).unwrap();
            ("sqlite", compiled.sql, compiled.params)
        },
    ] {
        let sub = "\"__ash_agg_children\"";
        assert!(
            sql.contains(&format!("{sub}.\"archived_at\" IS NULL AND {sub}.\"name\" <>")),
            "{dialect} aggregate must apply the read filter: {sql}"
        );
        assert!(
            sql.contains("rel_folders_1.\"name\" = ") && sql.contains("rel_folders_1.\"archived_at\" IS NULL"),
            "{dialect} related filter must apply the read filter: {sql}"
        );
        let values: Vec<&Value> = params.iter().map(|p| &p.value).collect();
        assert_eq!(
            values,
            [
                &Value::String("root".into()),
                &Value::String("docs".into()),
                &Value::String("hidden".into()),
                &Value::String("hidden".into()),
            ]
        );
        if dialect == "postgres" {
            for n in 1..=4 {
                assert!(sql.contains(&format!("${n}")), "missing ${n}: {sql}");
            }
            assert!(!sql.contains("$5"), "placeholders must be consecutive: {sql}");
        }
    }
}

/// A count is `COUNT(*)` over the filtered table, or over the page when the query has a
/// limit or offset, and never sorts.
#[test]
fn test_count_compilation() {
    let dialect = PostgresDialect;
    let query = CompiledQuery {
        filter: Some(Filter::eq("status", "open")),
        sort: vec![Sort {
            field: "priority".into(),
            descending: true,
            guard: None,
        }],
        ..CompiledQuery::default()
    };
    let compiled = QueryCompiler::new(&dialect).compile_count(&TICKET_DEF, &query).unwrap();
    assert_eq!(compiled.sql(), r#"SELECT COUNT(*) FROM "tickets" WHERE "status" = $1"#);

    let page = CompiledQuery {
        limit: Some(10),
        offset: Some(20),
        ..query
    };
    let compiled = QueryCompiler::new(&dialect).compile_count(&TICKET_DEF, &page).unwrap();
    assert_eq!(
        compiled.sql(),
        r#"SELECT COUNT(*) FROM (SELECT 1 FROM "tickets" WHERE "status" = $1 LIMIT $2 OFFSET $3) AS counted"#
    );

    let offset_only = CompiledQuery {
        offset: Some(5),
        ..CompiledQuery::default()
    };
    let compiled = QueryCompiler::new(&SqliteDialect).compile_count(&TICKET_DEF, &offset_only).unwrap();
    assert_eq!(compiled.sql(), r#"SELECT COUNT(*) FROM (SELECT 1 FROM "tickets" LIMIT ? OFFSET ?) AS counted"#);
}

static ITEM_NAME: Expr = Expr::Field("name");

static BIN_DEF: ResourceDef = ResourceDef {
    name: "Bin",
    table: "bins",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    relationships: &[ash_core::RelationshipDef::has_many("items", || &ITEM_DEF, "bin_id")],
    actions: &[ActionDef::read("read").primary()],
    aggregates: &[],
    calculations: &[],
    ..FOLDER_DEF
};

static ITEM_DEF: ResourceDef = ResourceDef {
    name: "Item",
    table: "items",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("bin_id", AttrType::Uuid),
        AttributeDef::required("name", AttrType::String),
    ],
    relationships: &[],
    actions: &[ActionDef::read("read").primary()],
    aggregates: &[],
    calculations: &[CalculationDef::new("loud_name", AttrType::String, Expr::Upper(&ITEM_NAME))],
    ..FOLDER_DEF
};

/// A calculation in a filter on related records reads that subquery's table, by its
/// alias, as the filter's fields do: both tables have a `name`.
#[test]
fn a_related_calculation_reads_its_own_table() {
    let query = CompiledQuery {
        filter: Some(Filter::related("items", Filter::eq("loud_name", "BOLT"))),
        ..CompiledQuery::default()
    };
    let mut compiler = QueryCompiler::new(&PostgresDialect);
    let sql = compiler.compile_select(&BIN_DEF, &query).unwrap().sql;
    let alias = sql.split("\"items\" AS ").nth(1).and_then(|rest| rest.split_whitespace().next()).expect("an alias");
    assert!(sql.contains(&format!("upper({alias}.\"name\")")), "{sql}");
}
