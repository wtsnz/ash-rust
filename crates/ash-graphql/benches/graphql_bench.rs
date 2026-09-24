use ash_core::{
    ActionDef, AttrType, AttributeDef, Context, DataLayer, FieldMap, OnDelete, RelKind,
    RelationshipDef, ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use async_graphql::Request;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tower::ServiceExt;
use uuid::Uuid;

static USER_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::required("email", AttrType::String),
];

static USER_RELS: &[RelationshipDef] = &[RelationshipDef {
    name: "tickets",
    kind: RelKind::HasMany,
    destination: || &TICKET_DEF,
    source_attribute: "id",
    destination_attribute: "author_id",
    through: None,
    source_attribute_on_join_resource: None,
    destination_attribute_on_join_resource: None,
    on_delete: OnDelete::Cascade,
}];

static USER_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("create")
        .primary()
        .accept(&["name", "email"]),
];

static USER_DEF: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: USER_ATTRS,
    relationships: USER_RELS,
    actions: USER_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["OPEN", "CLOSED"],
        },
    ),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::required("author_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[RelationshipDef {
    name: "author",
    kind: RelKind::BelongsTo,
    destination: || &USER_DEF,
    source_attribute: "author_id",
    destination_attribute: "id",
    through: None,
    source_attribute_on_join_resource: None,
    destination_attribute_on_join_resource: None,
    on_delete: OnDelete::Nothing,
}];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("open")
        .primary()
        .accept(&["title", "status", "priority", "author_id"]),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: TICKET_RELS,
    actions: TICKET_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static SQLITE_USER_DEF: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: USER_ATTRS,
    relationships: USER_RELS,
    actions: USER_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static SQLITE_TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: TICKET_RELS,
    actions: TICKET_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn setup_memory_benchmark_data(
    mem: &Memory,
) -> (Context<Memory>, Uuid, Vec<Uuid>, async_graphql::dynamic::Schema) {
    let ctx = Context::new(mem.clone());
    let mut author_ids = Vec::new();

    // Create 5 authors
    for i in 1..=5 {
        let author_id = Uuid::new_v4();
        let mut u = FieldMap::new();
        u.insert("id".into(), Value::Uuid(author_id));
        u.insert("name".into(), Value::String(format!("Engineer #{i}")));
        u.insert("email".into(), Value::String(format!("engineer{i}@example.com")));
        mem.create(&USER_DEF, author_id, u).await.unwrap();
        author_ids.push(author_id);
    }

    let mut first_ticket_id = Uuid::nil();

    // Create 100 tickets distributed across the 5 authors
    for i in 1..=100 {
        let t_id = Uuid::new_v4();
        if i == 1 {
            first_ticket_id = t_id;
        }
        let author_id = author_ids[(i - 1) % author_ids.len()];
        let status = if i % 2 == 0 { "OPEN" } else { "CLOSED" };
        let priority = (i % 5) as i64 + 1;

        let mut t = FieldMap::new();
        t.insert("id".into(), Value::Uuid(t_id));
        t.insert("title".into(), Value::String(format!("Issue #{i}: System performance degradation")));
        t.insert("status".into(), Value::String(status.into()));
        t.insert("priority".into(), Value::Int(priority));
        t.insert("author_id".into(), Value::Uuid(author_id));

        mem.create(&TICKET_DEF, t_id, t).await.unwrap();
    }

    let schema = AshGraphQL::from_resources(&[&USER_DEF, &TICKET_DEF])
        .with_dataloader()
        .finish_with_context(ctx.clone())
        .expect("schema build");

    (ctx, first_ticket_id, author_ids, schema)
}

fn bench_graphql_schema_build(c: &mut Criterion) {
    let mut group = c.benchmark_group("graphql_schema");

    group.bench_function("schema_reflection_memory", |b| {
        b.iter(|| {
            let schema = AshGraphQL::from_resources(&[&USER_DEF, &TICKET_DEF])
                .with_dataloader()
                .finish::<Memory>();
            black_box(schema.unwrap());
        });
    });

    group.finish();
}

fn bench_graphql_queries(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mem = Memory::new();
    let (_ctx, sample_ticket_id, _, schema) = rt.block_on(setup_memory_benchmark_data(&mem));
    let mut group = c.benchmark_group("graphql_queries");

    // 1. Single Record Query
    let query_single = format!(
        r#"query {{ getTicket(id: "{}") {{ id title status priority }} }}"#,
        sample_ticket_id
    );
    group.bench_function("single_record_by_id", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(&query_single)).await;
            black_box(res);
        });
    });

    // 2. Collection Query (100 items)
    let query_collection = r#"query { listTickets(limit: 100) { id title status priority } }"#;
    group.bench_function("collection_100_records", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(query_collection)).await;
            black_box(res);
        });
    });

    // 3. Filtered & Sorted Query
    let query_filter_sort = r#"
        query {
            listTickets(
                filter: { status: { eq: OPEN } }
                sort: [{ field: PRIORITY, order: DESC }]
                limit: 50
            ) {
                id
                title
                priority
            }
        }
    "#;
    group.bench_function("filtered_and_sorted_query", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(query_filter_sort)).await;
            black_box(res);
        });
    });

    // 4. Relay Keyset Pagination
    let query_relay = r#"
        query {
            ticketsConnection(first: 20) {
                totalCount
                pageInfo {
                    hasNextPage
                    endCursor
                }
                edges {
                    cursor
                    node {
                        id
                        title
                    }
                }
            }
        }
    "#;
    group.bench_function("relay_keyset_pagination_20", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(query_relay)).await;
            black_box(res);
        });
    });

    // 5. DataLoader Nested Relationships (Resolving author for 100 tickets in 1 batched roundtrip)
    let query_nested = r#"
        query {
            listTickets(limit: 100) {
                id
                title
                author {
                    id
                    name
                    email
                }
            }
        }
    "#;
    group.bench_function("dataloader_nested_100_tickets", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(query_nested)).await;
            black_box(res);
        });
    });

    group.finish();
}

fn bench_graphql_mutations(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mem = Memory::new();
    let (_, _, author_ids, schema) = rt.block_on(setup_memory_benchmark_data(&mem));
    let mut group = c.benchmark_group("graphql_mutations");

    let author_id = author_ids[0];

    // Open ticket mutation
    let mutation = format!(
        r#"
        mutation {{
            openTicket(input: {{
                title: "Benchmarked Critical Failure",
                status: OPEN,
                priority: 1,
                author_id: "{author_id}"
            }}) {{
                result {{
                    id
                    title
                    status
                }}
                errors {{
                    field
                    message
                }}
            }}
        }}
        "#
    );

    group.bench_function("mutation_open_ticket", |b| {
        b.to_async(&rt).iter(|| async {
            let res = schema.execute(Request::new(&mutation)).await;
            black_box(res);
        });
    });

    group.finish();
}

fn bench_axum_http_roundtrip(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mem = Memory::new();
    let (_, _, _, schema) = rt.block_on(setup_memory_benchmark_data(&mem));
    let app = ash_graphql::axum::graphql_router(schema);
    let mut group = c.benchmark_group("graphql_http");

    let payload = serde_json::json!({
        "query": "query { listTickets(limit: 20) { id title status priority } }"
    })
    .to_string();

    group.bench_function("axum_http_post_graphql", |b| {
        b.to_async(&rt).iter(|| async {
            let req = axum::http::Request::builder()
                .method("POST")
                .uri("/graphql")
                .header("Content-Type", "application/json")
                .header("Content-Length", payload.len().to_string())
                .body(axum::body::Body::from(payload.clone()))
                .unwrap();

            let res = app.clone().oneshot(req).await.unwrap();
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            black_box(body);
        });
    });

    group.finish();
}

fn bench_sqlite_vs_memory(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("graphql_sqlite_vs_memory");

    // Setup SQLite
    let (_sqlite_ctx, sqlite_schema) = rt.block_on(async {
        let db = Sqlite::memory().await.unwrap();
        db.install(&[&SQLITE_USER_DEF, &SQLITE_TICKET_DEF])
            .await
            .unwrap();

        let author_id = Uuid::new_v4();
        let mut u = FieldMap::new();
        u.insert("id".into(), Value::Uuid(author_id));
        u.insert("name".into(), Value::String("SQLite Engineer".into()));
        u.insert("email".into(), Value::String("sqlite@example.com".into()));
        db.create(&SQLITE_USER_DEF, author_id, u).await.unwrap();

        for i in 1..=100 {
            let t_id = Uuid::new_v4();
            let mut t = FieldMap::new();
            t.insert("id".into(), Value::Uuid(t_id));
            t.insert("title".into(), Value::String(format!("SQLite Ticket #{i}")));
            t.insert("status".into(), Value::String("OPEN".into()));
            t.insert("priority".into(), Value::Int(1));
            t.insert("author_id".into(), Value::Uuid(author_id));
            db.create(&SQLITE_TICKET_DEF, t_id, t).await.unwrap();
        }

        let ctx = Context::new(db);
        let s = AshGraphQL::from_resources(&[&SQLITE_USER_DEF, &SQLITE_TICKET_DEF])
            .with_dataloader()
            .finish_with_context(ctx.clone())
            .unwrap();
        (ctx, s)
    });

    let query_100 = r#"query { listTickets(limit: 100) { id title status priority } }"#;

    group.bench_function("sqlite_list_100_tickets", |b| {
        b.to_async(&rt).iter(|| async {
            let res = sqlite_schema.execute(Request::new(query_100)).await;
            black_box(res);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_graphql_schema_build,
    bench_graphql_queries,
    bench_graphql_mutations,
    bench_axum_http_roundtrip,
    bench_sqlite_vs_memory,
);
criterion_main!(benches);
