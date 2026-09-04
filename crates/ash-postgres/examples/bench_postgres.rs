use std::time::{Duration, Instant};
use ash_core::{
    ActionDef, AggregateDef, AggregateFilter, AttrType, AttributeDef, CompiledQuery, ConstValue,
    DataLayer, FieldMap, Filter, RelationshipDef, ResourceDef, Sort, TransactionSupport, Value,
};
use ash_postgres::Postgres;
use uuid::Uuid;

static USER_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::required("email", AttrType::String),
];

static USER_RELS: &[RelationshipDef] = &[
    RelationshipDef::has_many("tickets", || &TICKET_DEF, "user_id"),
];

static USER_AGGS: &[AggregateDef] = &[
    AggregateDef::count("ticket_count", "tickets"),
    AggregateDef::count_where(
        "open_ticket_count",
        "tickets",
        AggregateFilter::Eq("status", ConstValue::Str("open")),
    ),
];

static USER_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["name", "email"]).primary(),
    ActionDef::read("read").primary(),
];

pub static USER_DEF: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: USER_ATTRS,
    relationships: USER_RELS,
    actions: USER_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: USER_AGGS,
    extensions: &[],
    notifiers: &[],
    identities: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("user_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[
    RelationshipDef::belongs_to("user", || &USER_DEF, "user_id"),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["title", "status", "priority", "user_id"])
        .primary(),
    ActionDef::read("read").primary(),
];

pub static TICKET_DEF: ResourceDef = ResourceDef {
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
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
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://ash_user:ash_password@localhost:5433/ash_benchmark".to_string()
    });

    println!("\n========================================================");
    println!("      ash-postgres Real-World Benchmark Runner          ");
    println!("========================================================");
    println!("Connecting to PostgreSQL: {database_url} ...\n");

    let pg = Postgres::connect(&database_url).await?;
    let pool = pg.pool().expect("Expected PostgreSQL connection pool");

    // Initialize clean schema
    sqlx::query("DROP TABLE IF EXISTS tickets CASCADE;").execute(pool).await?;
    sqlx::query("DROP TABLE IF EXISTS users CASCADE;").execute(pool).await?;

    sqlx::query(
        "CREATE TABLE users (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL
        );"
    ).execute(pool).await?;

    sqlx::query(
        "CREATE TABLE tickets (
            id UUID PRIMARY KEY,
            title TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'open',
            priority BIGINT NOT NULL DEFAULT 1,
            user_id UUID REFERENCES users(id)
        );"
    ).execute(pool).await?;

    // Seed 5 users
    let mut user_ids = Vec::new();
    for i in 1..=5 {
        let u_id = Uuid::new_v4();
        let mut u = FieldMap::new();
        u.insert("id".into(), Value::Uuid(u_id));
        u.insert("name".into(), Value::String(format!("Staff Engineer #{i}")));
        u.insert("email".into(), Value::String(format!("staff{i}@company.com")));
        pg.create(&USER_DEF, u_id, u).await?;
        user_ids.push(u_id);
    }
    let first_user_id = user_ids[0];

    // Seed 100 tickets
    let mut first_ticket_id = Uuid::nil();
    for i in 1..=100 {
        let t_id = Uuid::new_v4();
        if i == 1 {
            first_ticket_id = t_id;
        }
        let user_id = user_ids[(i - 1) % user_ids.len()];
        let status = if i % 2 == 0 { "open" } else { "closed" };
        let priority = (i % 5) as i64 + 1;

        let mut t = FieldMap::new();
        t.insert("id".into(), Value::Uuid(t_id));
        t.insert("title".into(), Value::String(format!("Incident #{i}: Database connection pool exhausted")));
        t.insert("status".into(), Value::String(status.into()));
        t.insert("priority".into(), Value::Int(priority));
        t.insert("user_id".into(), Value::Uuid(user_id));
        pg.create(&TICKET_DEF, t_id, t).await?;
    }

    let warmup = Duration::from_millis(500);
    let bench_dur = Duration::from_secs(2);

    // 1. Point Write: Ticket.create (Single-Roundtrip INSERT ... RETURNING *)
    run_bench(
        "1. Point Write: Ticket.open (RETURNING *)",
        warmup,
        bench_dur,
        || async {
            let t_id = Uuid::new_v4();
            let mut t = FieldMap::new();
            t.insert("id".into(), Value::Uuid(t_id));
            t.insert("title".into(), Value::String("Benchmarked Critical Failure".into()));
            t.insert("status".into(), Value::String("open".into()));
            t.insert("priority".into(), Value::Int(1));
            t.insert("user_id".into(), Value::Uuid(first_user_id));
            let res = pg.create(&TICKET_DEF, t_id, t).await.unwrap();
            assert_eq!(res.get("id"), Some(&Value::Uuid(t_id)));
        },
    )
    .await;

    // 2. Point Read: Ticket.get(id) (Primary Key Lookup)
    let get_query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(first_ticket_id))),
        limit: Some(1),
        ..CompiledQuery::default()
    };
    run_bench(
        "2. Point Read: Ticket.get(id) (Primary Key)",
        warmup,
        bench_dur,
        || async {
            let rows = pg.run_query(&TICKET_DEF, &get_query).await.unwrap();
            assert_eq!(rows.len(), 1);
        },
    )
    .await;

    // 3. Filtered & Sorted Query: 50 items (status == "open", priority DESC)
    let filter_query = CompiledQuery {
        filter: Some(Filter::eq("status", "open")),
        sort: vec![Sort {
            field: "priority".to_string(),
            descending: true,
        }],
        limit: Some(50),
        ..CompiledQuery::default()
    };
    run_bench(
        "3. Filtered & Sorted Query (50 items)",
        warmup,
        bench_dur,
        || async {
            let rows = pg.run_query(&TICKET_DEF, &filter_query).await.unwrap();
            assert!(!rows.is_empty());
        },
    )
    .await;

    // 4. Correlated Subquery Aggregates: User with ticket_count & open_ticket_count
    let agg_query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(first_user_id))),
        aggregates: vec!["ticket_count".to_string(), "open_ticket_count".to_string()],
        limit: Some(1),
        ..CompiledQuery::default()
    };
    run_bench(
        "4. Correlated Aggregates (Subqueries)",
        warmup,
        bench_dur,
        || async {
            let rows = pg.run_query(&USER_DEF, &agg_query).await.unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0].contains_key("ticket_count"));
            assert!(rows[0].contains_key("open_ticket_count"));
        },
    )
    .await;

    // 5. Bulk Ingestion: 100 Tickets (Transactional Batch Insert)
    run_bench(
        "5. Bulk Ingestion: 100 Tickets Batch",
        warmup,
        bench_dur,
        || async {
            let mut batch = Vec::with_capacity(100);
            for i in 1..=100 {
                let id = Uuid::new_v4();
                let mut f = FieldMap::new();
                f.insert("id".into(), Value::Uuid(id));
                f.insert("title".into(), Value::String(format!("Bulk ticket #{i}")));
                f.insert("status".into(), Value::String("open".into()));
                f.insert("priority".into(), Value::Int(2));
                f.insert("user_id".into(), Value::Uuid(first_user_id));
                batch.push((id, f));
            }
            let res = pg.bulk_create(&TICKET_DEF, batch).await.unwrap();
            assert_eq!(res.len(), 100);
        },
    )
    .await;

    // 6. Transactional Workflow: Atomic Multi-Step User + Ticket (BEGIN/COMMIT)
    run_bench(
        "6. Transactional Workflow: User + Ticket",
        warmup,
        bench_dur,
        || async {
            let res = pg.transaction(|tx| {
                let tx = tx.clone();
                async move {
                    let u_id = Uuid::new_v4();
                    let mut u = FieldMap::new();
                    u.insert("id".into(), Value::Uuid(u_id));
                    u.insert("name".into(), Value::String("Tx User".into()));
                    u.insert("email".into(), Value::String("tx@company.com".into()));
                    tx.create(&USER_DEF, u_id, u).await?;

                    let t_id = Uuid::new_v4();
                    let mut t = FieldMap::new();
                    t.insert("id".into(), Value::Uuid(t_id));
                    t.insert("title".into(), Value::String("Tx Ticket".into()));
                    t.insert("status".into(), Value::String("open".into()));
                    t.insert("priority".into(), Value::Int(1));
                    t.insert("user_id".into(), Value::Uuid(u_id));
                    tx.create(&TICKET_DEF, t_id, t).await?;

                    Ok((u_id, t_id))
                }
            })
            .await
            .unwrap();

            assert_ne!(res.0, Uuid::nil());
            assert_ne!(res.1, Uuid::nil());
        },
    )
    .await;

    println!("========================================================");
    println!("         PostgreSQL Benchmark Suite Complete!           ");
    println!("========================================================\n");

    Ok(())
}
