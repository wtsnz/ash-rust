use ash_memory::Memory;
use ash_sqlite::Sqlite;
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use helpdesk::representative::fields as r;
use helpdesk::ticket::fields as t;
use helpdesk::{Helpdesk, Representative, Ticket, actor_customer, actor_representative};
use uuid::Uuid;

fn bench_actions(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut group = c.benchmark_group("actions");

    // 1. Ticket.open with validation + changeset + in-memory store
    group.bench_function("ticket_open_memory", |b| {
        let desk = Helpdesk::new(Memory::new());
        let customer = desk.with_actor(actor_customer(Uuid::new_v4()));
        b.to_async(&rt).iter(|| async {
            let res = customer.open_ticket(black_box("Printer is broken")).await;
            black_box(res.unwrap());
        });
    });

    // 2. Representative.create with validation + in-memory store
    group.bench_function("representative_create_memory", |b| {
        let desk = Helpdesk::new(Memory::new());
        b.to_async(&rt).iter(|| async {
            let res = desk.create_representative(black_box("Alice Smith")).await;
            black_box(res.unwrap());
        });
    });

    // 3. Ticket.open with SQLite data layer
    group.bench_function("ticket_open_sqlite", |b| {
        let desk = rt.block_on(async {
            let db = Sqlite::memory().await.unwrap();
            let d = Helpdesk::new(db);
            d.install().await.unwrap();
            d
        });
        let customer = desk.with_actor(actor_customer(Uuid::new_v4()));
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
    let desk_mem = Helpdesk::new(Memory::new());
    let customer_mem = desk_mem.with_actor(actor_customer(Uuid::new_v4()));
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
                .filter(t::status.eq(black_box("open")))
                .all()
                .await;
            black_box(res.unwrap());
        });
    });

    // Pre-populate 100 tickets in SQLite
    let desk_sql = rt.block_on(async {
        let db = Sqlite::memory().await.unwrap();
        let d = Helpdesk::new(db);
        d.install().await.unwrap();
        let cust = d.with_actor(actor_customer(Uuid::new_v4()));
        for i in 1..=100 {
            cust.open_ticket(&format!("Ticket number {i}")).await.unwrap();
        }
        d
    });
    let customer_sql = desk_sql.with_actor(actor_customer(Uuid::new_v4()));

    group.bench_function("filter_status_open_100_sqlite", |b| {
        b.to_async(&rt).iter(|| async {
            let res = Ticket::query(&customer_sql)
                .filter(t::status.eq(black_box("open")))
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
    let desk_mem = Helpdesk::new(Memory::new());
    let rep_id_mem = Uuid::new_v4();
    let rep_actor_mem = actor_representative(rep_id_mem);
    let customer_mem = desk_mem.with_actor(actor_customer(Uuid::new_v4()));

    rt.block_on(async {
        desk_mem.create_representative("Bob Jones").await.unwrap();
        for i in 1..=20 {
            let ticket = customer_mem
                .open_ticket(&format!("Ticket {i}"))
                .await
                .unwrap();
            let as_rep = desk_mem.with_actor(rep_actor_mem.clone());
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
    let desk_sql = rt.block_on(async {
        let db = Sqlite::memory().await.unwrap();
        let d = Helpdesk::new(db);
        d.install().await.unwrap();
        d
    });
    let rep_id_sql = Uuid::new_v4();
    let rep_actor_sql = actor_representative(rep_id_sql);
    let customer_sql = desk_sql.with_actor(actor_customer(Uuid::new_v4()));

    rt.block_on(async {
        desk_sql.create_representative("Bob Jones").await.unwrap();
        for i in 1..=20 {
            let ticket = customer_sql
                .open_ticket(&format!("Ticket {i}"))
                .await
                .unwrap();
            let as_rep = desk_sql.with_actor(rep_actor_sql.clone());
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
