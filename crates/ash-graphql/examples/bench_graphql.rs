//! Fast Standalone GraphQL Benchmark Runner
//!
//! Run with:
//! ```bash
//! cargo run --release -p ash-graphql --example bench_graphql --features axum
//! ```

use std::time::{Duration, Instant};

use ash_core::{
    ActionDef, AttrType, AttributeDef, Context, DataLayer, FieldMap, OnDelete, OnUpdate, RelKind,
    RelationshipDef, ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use async_graphql::Request;
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
    on_update: OnUpdate::Nothing,
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
    checks: &[],
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
    on_update: OnUpdate::Nothing,
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
    checks: &[],
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
    checks: &[],
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
    checks: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Sqlite,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn run_bench<F, Fut>(name: &str, warmup_dur: Duration, bench_dur: Duration, mut op: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    print!("  • Warming up {name} ({}s)...", warmup_dur.as_secs());
    let warmup_start = Instant::now();
    let mut warmup_count = 0u64;
    while warmup_start.elapsed() < warmup_dur {
        op().await;
        warmup_count += 1;
    }
    println!(" done ({warmup_count} iterations).");

    print!("  • Benchmarking {name} ({}s)...", bench_dur.as_secs());
    let mut latencies = Vec::with_capacity(100_000);
    let start = Instant::now();
    while start.elapsed() < bench_dur {
        let op_start = Instant::now();
        op().await;
        latencies.push(op_start.elapsed());
    }
    let total_elapsed = start.elapsed();
    let count = latencies.len() as f64;
    let ips = count / total_elapsed.as_secs_f64();

    latencies.sort_unstable();
    let avg = total_elapsed / latencies.len() as u32;
    let median = latencies[latencies.len() / 2];
    let p95 = latencies[(latencies.len() as f64 * 0.95) as usize];
    let p99 = latencies[(latencies.len() as f64 * 0.99) as usize];

    println!(" completed!\n");
    println!("┌────────────────────────────────────────────────────────┐");
    println!("│ Benchmark: {:<43}│", name);
    println!("├────────────────────────────────────────────────────────┤");
    println!("│   Iterations:   {:>12}                       │", latencies.len());
    println!("│   Throughput:   {:>12.2} ops/sec                 │", ips);
    println!("│   Average:      {:>12.2} µs                      │", avg.as_nanos() as f64 / 1000.0);
    println!("│   Median (p50): {:>12.2} µs                      │", median.as_nanos() as f64 / 1000.0);
    println!("│   P95 Latency:  {:>12.2} µs                      │", p95.as_nanos() as f64 / 1000.0);
    println!("│   P99 Latency:  {:>12.2} µs                      │", p99.as_nanos() as f64 / 1000.0);
    println!("└────────────────────────────────────────────────────────┘\n");
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n========================================================");
    println!("      ash-graphql Real-World Benchmark Runner            ");
    println!("========================================================\n");

    let mem = Memory::new();
    let ctx = Context::new(mem.clone());

    // Seed 5 authors
    let mut author_ids = Vec::new();
    for i in 1..=5 {
        let u_id = Uuid::new_v4();
        let mut u = FieldMap::new();
        u.insert("id".into(), Value::Uuid(u_id));
        u.insert("name".into(), Value::String(format!("Staff Engineer #{i}")));
        u.insert("email".into(), Value::String(format!("staff{i}@company.com")));
        mem.create(&USER_DEF, u_id, u).await?;
        author_ids.push(u_id);
    }

    // Seed 100 tickets
    let mut first_ticket_id = Uuid::nil();
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
        t.insert("title".into(), Value::String(format!("Incident #{i}: Database connection pool exhausted")));
        t.insert("status".into(), Value::String(status.into()));
        t.insert("priority".into(), Value::Int(priority));
        t.insert("author_id".into(), Value::Uuid(author_id));
        mem.create(&TICKET_DEF, t_id, t).await?;
    }

    let schema = AshGraphQL::from_resources(&[&USER_DEF, &TICKET_DEF])
        .with_dataloader()
        .finish_with_context(ctx.clone())?;

    let warmup = Duration::from_millis(500);
    let bench_dur = Duration::from_secs(2);

    // 1. Schema Reflection Benchmark
    run_bench(
        "Schema Build (Dynamic Reflection)",
        warmup,
        bench_dur,
        || async {
            let _ = AshGraphQL::from_resources(&[&USER_DEF, &TICKET_DEF])
                .with_dataloader()
                .finish::<Memory>()
                .unwrap();
        },
    )
    .await;

    // 2. Single Record Query
    let single_query = format!(
        r#"query {{ getTicket(id: "{}") {{ id title status priority }} }}"#,
        first_ticket_id
    );
    run_bench(
        "GraphQL Query: Single Record by ID",
        warmup,
        bench_dur,
        || async {
            let res = schema.execute(Request::new(&single_query)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    // 3. Collection Query (100 records)
    let collection_query = r#"query { listTickets(limit: 100) { id title status priority } }"#;
    run_bench(
        "GraphQL Query: 100 Tickets Collection",
        warmup,
        bench_dur,
        || async {
            let res = schema.execute(Request::new(collection_query)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    // 4. Filtered & Sorted Query
    let filter_sort_query = r#"
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
    run_bench(
        "GraphQL Query: Filtered & Sorted (50 items)",
        warmup,
        bench_dur,
        || async {
            let res = schema.execute(Request::new(filter_sort_query)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    // 5. Relay Keyset Pagination
    let relay_query = r#"
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
    run_bench(
        "GraphQL Query: Relay Keyset Pagination (first: 20)",
        warmup,
        bench_dur,
        || async {
            let res = schema.execute(Request::new(relay_query)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    // 6. DataLoader Nested Relations (100 tickets + author resolved in 1 batch)
    let nested_query = r#"
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
    run_bench(
        "DataLoader: 100 Tickets + Nested Author",
        warmup,
        bench_dur,
        || async {
            let res = schema.execute(Request::new(nested_query)).await;
            assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
        },
    )
    .await;

    // 7. Full Axum HTTP POST Roundtrip (Routing + Deserialization + Execution + Serialization)
    let app = ash_graphql::axum::graphql_router(schema.clone());
    let http_payload = serde_json::json!({
        "query": "query { listTickets(limit: 20) { id title status priority } }"
    })
    .to_string();

    run_bench(
        "Axum HTTP POST /graphql Roundtrip (20 items)",
        warmup,
        bench_dur,
        || async {
            let req = axum::http::Request::builder()
                .method("POST")
                .uri("/graphql")
                .header("Content-Type", "application/json")
                .header("Content-Length", http_payload.len().to_string())
                .body(axum::body::Body::from(http_payload.clone()))
                .unwrap();

            let res = app.clone().oneshot(req).await.unwrap();
            let status = res.status();
            let body = axum::body::to_bytes(res.into_body(), usize::MAX)
                .await
                .unwrap();
            assert!(status.is_success());
            assert!(!body.is_empty());
        },
    )
    .await;

    // 8. GraphQL Mutation (Open Ticket with validation & changeset)
    let mut_mem = Memory::new();
    let mut_author_id = Uuid::new_v4();
    let mut mut_u = FieldMap::new();
    mut_u.insert("id".into(), Value::Uuid(mut_author_id));
    mut_u.insert("name".into(), Value::String("Mut Author".into()));
    mut_u.insert("email".into(), Value::String("mut@example.com".into()));
    mut_mem.create(&USER_DEF, mut_author_id, mut_u).await?;

    let mut_ctx = Context::new(mut_mem);
    let mut_schema = AshGraphQL::from_resources(&[&USER_DEF, &TICKET_DEF])
        .finish_with_context(mut_ctx)?;

    let mutation = format!(
        r#"
        mutation {{
            openTicket(input: {{
                title: "Production Alert: High Error Rate",
                status: OPEN,
                priority: 1,
                author_id: "{mut_author_id}"
            }}) {{
                result {{
                    id
                    title
                    status
                }}
            }}
        }}
        "#
    );
    run_bench(
        "GraphQL Mutation: openTicket (Validation + Action)",
        warmup,
        bench_dur,
        || async {
            let res = mut_schema.execute(Request::new(&mutation)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    // 9. SQLite 100 Tickets Query
    let sqlite_db = Sqlite::memory().await?;
    sqlite_db
        .install(&[&SQLITE_USER_DEF, &SQLITE_TICKET_DEF])
        .await?;

    let sql_author_id = Uuid::new_v4();
    let mut u = FieldMap::new();
    u.insert("id".into(), Value::Uuid(sql_author_id));
    u.insert("name".into(), Value::String("Database Admin".into()));
    u.insert("email".into(), Value::String("dba@company.com".into()));
    sqlite_db.create(&SQLITE_USER_DEF, sql_author_id, u).await?;

    for i in 1..=100 {
        let t_id = Uuid::new_v4();
        let mut t = FieldMap::new();
        t.insert("id".into(), Value::Uuid(t_id));
        t.insert("title".into(), Value::String(format!("SQLite Ticket #{i}")));
        t.insert("status".into(), Value::String("OPEN".into()));
        t.insert("priority".into(), Value::Int(1));
        t.insert("author_id".into(), Value::Uuid(sql_author_id));
        sqlite_db.create(&SQLITE_TICKET_DEF, t_id, t).await?;
    }

    let sqlite_ctx = Context::new(sqlite_db);
    let sqlite_schema = AshGraphQL::from_resources(&[&SQLITE_USER_DEF, &SQLITE_TICKET_DEF])
        .with_dataloader()
        .finish_with_context(sqlite_ctx)?;

    run_bench(
        "SQLite DataLayer: 100 Tickets Query",
        warmup,
        bench_dur,
        || async {
            let res = sqlite_schema.execute(Request::new(collection_query)).await;
            assert!(res.errors.is_empty());
        },
    )
    .await;

    println!("========================================================");
    println!("         GraphQL Benchmark Suite Complete!              ");
    println!("========================================================\n");

    Ok(())
}
