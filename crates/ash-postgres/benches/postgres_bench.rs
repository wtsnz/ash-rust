use ash_core::{
    ActionDef, AggregateDef, AggregateFilter, AttrType, AttributeDef, CompiledQuery, ConstValue,
    FieldMap, Filter, RelationshipDef, ResourceDef, Sort, Value,
};
use ash_postgres::Postgres;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use uuid::Uuid;

static USER_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::required("email", AttrType::String),
];

static USER_RELS: &[RelationshipDef] = &[RelationshipDef::has_many(
    "tickets",
    || &TICKET_DEF,
    "user_id",
)];

static USER_AGGS: &[AggregateDef] = &[
    AggregateDef::count("ticket_count", "tickets"),
    AggregateDef::count_where(
        "open_ticket_count",
        "tickets",
        AggregateFilter::Eq("status", ConstValue::Str("open")),
    ),
];

static USER_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create")
        .accept(&["id", "name", "email"])
        .primary(),
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

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("user_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] =
    &[RelationshipDef::belongs_to("user", || &USER_DEF, "user_id")];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["id", "title", "status", "priority", "user_id"])
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

async fn setup_postgres() -> Option<(Postgres, Uuid, Uuid)> {
    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://ash_user:ash_password@localhost:5433/ash_benchmark".to_string()
    });

    let pg = Postgres::connect(&database_url).await.ok()?;

    let _ = pg
        .execute_sql("DROP TABLE IF EXISTS tickets CASCADE;")
        .await;
    let _ = pg.execute_sql("DROP TABLE IF EXISTS users CASCADE;").await;

    pg.execute_sql(
        "CREATE TABLE users (
            id UUID PRIMARY KEY,
            name TEXT NOT NULL,
            email TEXT NOT NULL
        );",
    )
    .await
    .ok()?;

    pg.execute_sql(
        "CREATE TABLE tickets (
            id UUID PRIMARY KEY,
            title TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'open',
            priority BIGINT NOT NULL DEFAULT 1,
            user_id UUID REFERENCES users(id)
        );",
    )
    .await
    .ok()?;

    let ctx = ash_core::Runtime::new(pg.clone()).system().bootstrap();
    // Seed 5 users
    let mut user_ids = Vec::new();
    for i in 1..=5 {
        let u_id = Uuid::new_v4();
        let mut u = FieldMap::new();
        u.insert("id".into(), Value::Uuid(u_id));
        u.insert("name".into(), Value::String(format!("Staff Engineer #{i}")));
        u.insert(
            "email".into(),
            Value::String(format!("staff{i}@company.com")),
        );
        let _ =
            ash_core::create_dynamic(&ctx, &USER_DEF, USER_DEF.action("create").unwrap(), u).await;
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
        t.insert(
            "title".into(),
            Value::String(format!("Incident #{i}: Database connection pool exhausted")),
        );
        t.insert("status".into(), Value::String(status.into()));
        t.insert("priority".into(), Value::Int(priority));
        t.insert("user_id".into(), Value::Uuid(user_id));
        let _ = ash_core::create_dynamic(&ctx, &TICKET_DEF, TICKET_DEF.action("open").unwrap(), t)
            .await;
    }

    Some((pg, first_user_id, first_ticket_id))
}

fn bench_postgres_operations(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let Some((pg, user_id, ticket_id)) = rt.block_on(setup_postgres()) else {
        eprintln!("PostgreSQL not reachable; skipping benchmark");
        return;
    };

    let ctx = ash_core::Runtime::new(pg.clone()).system().bootstrap();
    let mut group = c.benchmark_group("postgres");

    // 1. Point Write
    group.bench_function("point_write_returning", |b| {
        b.to_async(&rt).iter(|| async {
            let t_id = Uuid::new_v4();
            let mut t = FieldMap::new();
            t.insert("id".into(), Value::Uuid(t_id));
            t.insert(
                "title".into(),
                Value::String("Benchmarked Critical Failure".into()),
            );
            t.insert("status".into(), Value::String("open".into()));
            t.insert("priority".into(), Value::Int(1));
            t.insert("user_id".into(), Value::Uuid(user_id));
            let res =
                ash_core::create_dynamic(&ctx, &TICKET_DEF, TICKET_DEF.action("open").unwrap(), t)
                    .await
                    .unwrap();
            black_box(res);
        });
    });

    // 2. Point Read
    let get_query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(ticket_id))),
        limit: Some(1),
        ..CompiledQuery::default()
    };
    group.bench_function("point_read_pk", |b| {
        b.to_async(&rt).iter(|| async {
            let rows = read(&ctx, &TICKET_DEF, &get_query).await.unwrap();
            black_box(rows);
        });
    });

    // 3. Filtered & Sorted
    let filter_query = CompiledQuery {
        filter: Some(Filter::eq("status", "open")),
        sort: vec![Sort {
            field: "priority".to_string(),
            descending: true,
            ..Default::default()
        }],
        limit: Some(50),
        ..CompiledQuery::default()
    };
    group.bench_function("filtered_sorted_50", |b| {
        b.to_async(&rt).iter(|| async {
            let rows = read(&ctx, &TICKET_DEF, &filter_query).await.unwrap();
            black_box(rows);
        });
    });

    // 4. Aggregates Subqueries
    let agg_query = CompiledQuery {
        filter: Some(Filter::eq("id", Value::Uuid(user_id))),
        aggregates: vec!["ticket_count".to_string(), "open_ticket_count".to_string()],
        limit: Some(1),
        ..CompiledQuery::default()
    };
    group.bench_function("aggregates_subqueries", |b| {
        b.to_async(&rt).iter(|| async {
            let rows = read(&ctx, &USER_DEF, &agg_query).await.unwrap();
            black_box(rows);
        });
    });

    // 5. Scoped action ingestion in one transaction (100 items)
    group.bench_function("scoped_action_batch_100", |b| {
        b.to_async(&rt).iter(|| async {
            let mut batch = Vec::with_capacity(100);
            for i in 1..=100 {
                let id = Uuid::new_v4();
                let mut f = FieldMap::new();
                f.insert("id".into(), Value::Uuid(id));
                f.insert("title".into(), Value::String(format!("Bulk ticket #{i}")));
                f.insert("status".into(), Value::String("open".into()));
                f.insert("priority".into(), Value::Int(2));
                f.insert("user_id".into(), Value::Uuid(user_id));
                batch.push((Value::Uuid(id), f));
            }
            let res = create_batch(&ctx, &TICKET_DEF, batch).await.unwrap();
            black_box(res);
        });
    });

    // 6. Transactional Workflow
    group.bench_function("transaction_multi_step", |b| {
        b.to_async(&rt).iter(|| async {
            let res = ctx
                .transaction(|tx| {
                    let tx = tx.clone();
                    async move {
                        let u_id = Uuid::new_v4();
                        let mut u = FieldMap::new();
                        u.insert("id".into(), Value::Uuid(u_id));
                        u.insert("name".into(), Value::String("Tx User".into()));
                        u.insert("email".into(), Value::String("tx@company.com".into()));
                        ash_core::create_dynamic(
                            &tx,
                            &USER_DEF,
                            USER_DEF.action("create").unwrap(),
                            u,
                        )
                        .await?;

                        let t_id = Uuid::new_v4();
                        let mut t = FieldMap::new();
                        t.insert("id".into(), Value::Uuid(t_id));
                        t.insert("title".into(), Value::String("Tx Ticket".into()));
                        t.insert("status".into(), Value::String("open".into()));
                        t.insert("priority".into(), Value::Int(1));
                        t.insert("user_id".into(), Value::Uuid(u_id));
                        ash_core::create_dynamic(
                            &tx,
                            &TICKET_DEF,
                            TICKET_DEF.action("open").unwrap(),
                            t,
                        )
                        .await?;

                        Ok((u_id, t_id))
                    }
                })
                .await
                .unwrap();
            black_box(res);
        });
    });

    group.finish();
}

criterion_group!(benches, bench_postgres_operations);
criterion_main!(benches);

async fn read(
    ctx: &ash_core::Context<Postgres>,
    resource: &ResourceDef,
    query: &CompiledQuery,
) -> ash_core::Result<Vec<ash_core::View>> {
    ctx.access()
        .read(
            resource,
            "read",
            &FieldMap::new(),
            ash_core::ReadRequest {
                filter: query.filter.clone(),
                sort: query.sort.clone(),
                limit: query.limit,
                offset: query.offset,
                aggregates: query.aggregates.clone(),
                calculations: query.calculations.clone(),
                ..Default::default()
            },
        )
        .await
}

async fn create_batch(
    ctx: &ash_core::Context<Postgres>,
    resource: &'static ResourceDef,
    rows: Vec<(Value, FieldMap)>,
) -> ash_core::Result<Vec<ash_core::View>> {
    ctx.transaction(|tx| async move {
        let mut result = Vec::with_capacity(rows.len());
        for (_, fields) in rows {
            result.push(
                ash_core::create_dynamic(&tx, resource, resource.action("open").unwrap(), fields)
                    .await?,
            );
        }
        Ok(result)
    })
    .await
}
