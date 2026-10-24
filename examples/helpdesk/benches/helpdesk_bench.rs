use ash_memory::Memory;
use ash_sqlite::Sqlite;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use helpdesk::representative::fields as r;
use helpdesk::ticket::{Status, fields as t};
use helpdesk::{Helpdesk, Representative, Ticket, actor_customer, actor_representative};
use uuid::Uuid;

fn bench_actions(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("actions");

    // 1. Ticket.open with validation + changeset + in-memory store
    group.bench_function("ticket_open_memory", |b| {
        let desk_runtime = ash_core::Runtime::new(Memory::new());
        let customer =
            Helpdesk::from_context(desk_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));
        b.to_async(&rt).iter(|| async {
            let res = customer.open_ticket(black_box("Printer is broken")).await;
            black_box(res.unwrap());
        });
    });

    // 2. Representative.create with validation + in-memory store
    group.bench_function("representative_create_memory", |b| {
        let desk_runtime = ash_core::Runtime::new(Memory::new());
        let desk = Helpdesk::from_context(desk_runtime.bootstrap());
        b.to_async(&rt).iter(|| async {
            let res = desk.create_representative(black_box("Alice Smith")).await;
            black_box(res.unwrap());
        });
    });

    // 3. Ticket.open with SQLite data layer
    group.bench_function("ticket_open_sqlite", |b| {
        let (desk_runtime, _desk) = rt.block_on(async {
            let db = Sqlite::memory().await.unwrap();
            let d_runtime = ash_core::Runtime::new(db);
            let d = Helpdesk::from_context(d_runtime.bootstrap());
            d.install().await.unwrap();
            (d_runtime, d)
        });
        let customer =
            Helpdesk::from_context(desk_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));
        b.to_async(&rt).iter(|| async {
            let res = customer.open_ticket(black_box("Printer is broken")).await;
            black_box(res.unwrap());
        });
    });

    group.finish();
}

fn bench_queries(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("queries");

    // Pre-populate 100 tickets in memory
    let desk_mem_runtime = ash_core::Runtime::new(Memory::new());
    let customer_mem =
        Helpdesk::from_context(desk_mem_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));
    rt.block_on(async {
        for i in 1..=100 {
            customer_mem
                .open_ticket(&format!("Ticket number {i}"))
                .await
                .unwrap();
        }
    });

    group.bench_function("filter_status_open_100_memory", |b| {
        b.to_async(&rt).iter(|| async {
            let res = Ticket::query(&customer_mem)
                .filter(t::status.eq(black_box(Status::Open)))
                .all()
                .await;
            black_box(res.unwrap());
        });
    });

    // Pre-populate 100 tickets in SQLite
    let (desk_sql_runtime, _desk_sql) = rt.block_on(async {
        let db = Sqlite::memory().await.unwrap();
        let d_runtime = ash_core::Runtime::new(db);
        let d = Helpdesk::from_context(d_runtime.bootstrap());
        d.install().await.unwrap();
        let cust =
            Helpdesk::from_context(d_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));
        for i in 1..=100 {
            cust.open_ticket(&format!("Ticket number {i}"))
                .await
                .unwrap();
        }
        (d_runtime, d)
    });
    let customer_sql =
        Helpdesk::from_context(desk_sql_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));

    group.bench_function("filter_status_open_100_sqlite", |b| {
        b.to_async(&rt).iter(|| async {
            let res = Ticket::query(&customer_sql)
                .filter(t::status.eq(black_box(Status::Open)))
                .all()
                .await;
            black_box(res.unwrap());
        });
    });

    group.finish();
}

fn bench_aggregates(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("aggregates");

    // Memory setup
    let desk_mem_runtime = ash_core::Runtime::new(Memory::new());
    let desk_mem = Helpdesk::from_context(desk_mem_runtime.bootstrap());
    let rep_id_mem = Uuid::new_v4();
    let rep_actor_mem = actor_representative(rep_id_mem);
    let customer_mem =
        Helpdesk::from_context(desk_mem_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));

    rt.block_on(async {
        desk_mem.create_representative("Bob Jones").await.unwrap();
        for i in 1..=20 {
            let ticket = customer_mem
                .open_ticket(&format!("Ticket {i}"))
                .await
                .unwrap();
            let as_rep =
                Helpdesk::from_context(desk_mem_runtime.bind(Some(rep_actor_mem.clone()), None));
            as_rep.assign_ticket(&ticket, rep_id_mem).await.unwrap();
        }
    });

    group.bench_function("load_aggregates_memory", |b| {
        b.to_async(&rt).iter(|| async {
            let res = Representative::query(&desk_mem)
                .aggregate(r::ticket_count)
                .aggregate(r::open_ticket_count)
                .aggregate(r::has_tickets)
                .all()
                .await;
            black_box(res.unwrap());
        });
    });

    // SQLite setup
    let (desk_sql_runtime, desk_sql) = rt.block_on(async {
        let db = Sqlite::memory().await.unwrap();
        let d_runtime = ash_core::Runtime::new(db);
        let d = Helpdesk::from_context(d_runtime.bootstrap());
        d.install().await.unwrap();
        (d_runtime, d)
    });
    let rep_id_sql = Uuid::new_v4();
    let rep_actor_sql = actor_representative(rep_id_sql);
    let customer_sql =
        Helpdesk::from_context(desk_sql_runtime.bind(Some(actor_customer(Uuid::new_v4())), None));

    rt.block_on(async {
        desk_sql.create_representative("Bob Jones").await.unwrap();
        for i in 1..=20 {
            let ticket = customer_sql
                .open_ticket(&format!("Ticket {i}"))
                .await
                .unwrap();
            let as_rep =
                Helpdesk::from_context(desk_sql_runtime.bind(Some(rep_actor_sql.clone()), None));
            as_rep.assign_ticket(&ticket, rep_id_sql).await.unwrap();
        }
    });

    group.bench_function("load_aggregates_sqlite", |b| {
        b.to_async(&rt).iter(|| async {
            let res = Representative::query(&desk_sql)
                .aggregate(r::ticket_count)
                .aggregate(r::open_ticket_count)
                .aggregate(r::has_tickets)
                .all()
                .await;
            black_box(res.unwrap());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_actions, bench_queries, bench_aggregates);
criterion_main!(benches);
