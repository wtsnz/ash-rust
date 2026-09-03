use helpdesk::representative::fields as r;
use helpdesk::{
    Helpdesk, Representative, actor_customer, actor_representative,
};
use uuid::Uuid;

mod order {
    use ash_core::resource;
    use uuid::Uuid;

    use super::line_item::LineItem;

    resource! {
        resource Order;
        table "orders";

        attributes {
            id: Uuid [pk],
            customer_name: String,
        }

        relationships {
            has_many line_items: Vec<LineItem> [fk: "order_id"],
        }

        aggregates {
            item_count: Option<i64> = count(line_items),
            total_amount: Option<i64> = sum(line_items, amount),
            paid_amount: Option<i64> = sum(line_items, amount, filter: status == "paid"),
            has_items: Option<bool> = exists(line_items),
            pending_item_sku: Option<String> = first(line_items, sku, filter: status == "pending"),
        }

        actions {
            create create {
                accept {
                    customer_name: String,
                }
            }
            read read {
                primary
            }
        }

        policies {
            policy always {
                authorize_if always
            }
        }
    }
}

mod line_item {
    use ash_core::resource;
    use uuid::Uuid;

    use super::order::Order;

    resource! {
        resource LineItem;
        table "line_items";

        attributes {
            id: Uuid [pk],
            order_id: Uuid,
            sku: String,
            amount: i64,
            status: String,
        }

        relationships {
            belongs_to order: Option<Order>,
        }

        actions {
            create create {
                accept {
                    order_id: Uuid,
                    sku: String,
                    amount: i64,
                    status: String,
                }
            }
            read read {
                primary
            }
        }

        policies {
            policy always {
                authorize_if always
            }
        }
    }
}

use line_item::LineItem;
use order::Order;

#[tokio::test]
async fn memory_aggregates_loading_and_filtering() {
    let mem = ash_memory::Memory::new();
    let desk = Helpdesk::new(mem);

    let alice = desk.create_representative("Alice").await.unwrap();
    let bob = desk.create_representative("Bob").await.unwrap();
    let _carol = desk.create_representative("Carol").await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    let as_alice = desk.with_actor(actor_representative(alice.id));

    // Alice gets 2 tickets (1 open, 1 closed)
    let t1 = as_customer.open_ticket("Printer broke").await.unwrap();
    as_alice.assign_ticket(&t1, alice.id).await.unwrap();

    let t2 = as_customer.open_ticket("Wifi down").await.unwrap();
    let t2_assigned = as_alice.assign_ticket(&t2, alice.id).await.unwrap();
    as_alice.close_ticket(&t2_assigned).await.unwrap();

    // Bob gets 1 ticket (open)
    let as_bob = desk.with_actor(actor_representative(bob.id));
    let t3 = as_customer.open_ticket("Need new monitor").await.unwrap();
    as_bob.assign_ticket(&t3, bob.id).await.unwrap();

    // Carol has 0 tickets

    // 1. Unloaded aggregates are None
    let reps = Representative::query(&as_customer)
        .sort(r::name)
        .all()
        .await
        .unwrap();
    assert_eq!(reps.len(), 3);
    assert_eq!(reps[0].ticket_count, None);
    assert_eq!(reps[0].has_tickets, None);

    // 2. Load aggregates via .aggregate()
    let reps_with_aggs = Representative::query(&as_customer)
        .aggregate(r::ticket_count)
        .aggregate(r::open_ticket_count)
        .aggregate(r::has_tickets)
        .aggregate(r::first_ticket_subject)
        .sort(r::name)
        .all()
        .await
        .unwrap();

    // Alice: 2 total, 1 open, has_tickets = true, first_ticket_subject is present
    let rep_alice = &reps_with_aggs[0];
    assert_eq!(rep_alice.name, "Alice");
    assert_eq!(rep_alice.ticket_count, Some(2));
    assert_eq!(rep_alice.open_ticket_count, Some(1));
    assert_eq!(rep_alice.has_tickets, Some(true));
    assert!(rep_alice.first_ticket_subject.is_some());

    // Bob: 1 total, 1 open, has_tickets = true, first_ticket_subject = "Need new monitor"
    let rep_bob = &reps_with_aggs[1];
    assert_eq!(rep_bob.name, "Bob");
    assert_eq!(rep_bob.ticket_count, Some(1));
    assert_eq!(rep_bob.open_ticket_count, Some(1));
    assert_eq!(rep_bob.has_tickets, Some(true));
    assert_eq!(
        rep_bob.first_ticket_subject.as_deref(),
        Some("Need new monitor")
    );

    // Carol: 0 total, 0 open, has_tickets = false, first_ticket_subject = None
    let rep_carol = &reps_with_aggs[2];
    assert_eq!(rep_carol.name, "Carol");
    assert_eq!(rep_carol.ticket_count, Some(0));
    assert_eq!(rep_carol.open_ticket_count, Some(0));
    assert_eq!(rep_carol.has_tickets, Some(false));
    assert_eq!(rep_carol.first_ticket_subject, None);

    // 3. Filter by aggregate (ticket_count > 0)
    let active_reps = Representative::query(&as_customer)
        .aggregate(r::ticket_count)
        .filter(r::ticket_count.gt(0_i64))
        .sort_desc(r::ticket_count)
        .all()
        .await
        .unwrap();
    assert_eq!(active_reps.len(), 2);
    assert_eq!(active_reps[0].name, "Alice");
    assert_eq!(active_reps[0].ticket_count, Some(2));
    assert_eq!(active_reps[1].name, "Bob");
    assert_eq!(active_reps[1].ticket_count, Some(1));
}

#[tokio::test]
async fn sqlite_aggregates_loading_and_filtering() {
    let db = ash_sqlite::Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(db);
    desk.install().await.unwrap();

    let alice = desk.create_representative("Alice").await.unwrap();
    let bob = desk.create_representative("Bob").await.unwrap();
    let _carol = desk.create_representative("Carol").await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    let as_alice = desk.with_actor(actor_representative(alice.id));

    // Alice gets 2 tickets (1 open, 1 closed)
    let t1 = as_customer.open_ticket("Printer broke").await.unwrap();
    as_alice.assign_ticket(&t1, alice.id).await.unwrap();

    let t2 = as_customer.open_ticket("Wifi down").await.unwrap();
    let t2_assigned = as_alice.assign_ticket(&t2, alice.id).await.unwrap();
    as_alice.close_ticket(&t2_assigned).await.unwrap();

    // Bob gets 1 ticket (open)
    let as_bob = desk.with_actor(actor_representative(bob.id));
    let t3 = as_customer.open_ticket("Keyboard missing").await.unwrap();
    as_bob.assign_ticket(&t3, bob.id).await.unwrap();

    // Query across SQLite using domain accessors
    let reps = desk
        .representatives()
        .aggregate(r::ticket_count)
        .aggregate(r::open_ticket_count)
        .aggregate(r::has_tickets)
        .aggregate(r::first_ticket_subject)
        .sort(r::name)
        .all()
        .await
        .unwrap();

    assert_eq!(reps.len(), 3);

    // Alice in SQLite
    let rep_alice = &reps[0];
    assert_eq!(rep_alice.name, "Alice");
    assert_eq!(rep_alice.ticket_count, Some(2));
    assert_eq!(rep_alice.open_ticket_count, Some(1));
    assert_eq!(rep_alice.has_tickets, Some(true));
    assert!(rep_alice.first_ticket_subject.is_some());

    // Bob in SQLite
    let rep_bob = &reps[1];
    assert_eq!(rep_bob.name, "Bob");
    assert_eq!(rep_bob.ticket_count, Some(1));
    assert_eq!(rep_bob.open_ticket_count, Some(1));
    assert_eq!(rep_bob.has_tickets, Some(true));
    assert_eq!(
        rep_bob.first_ticket_subject.as_deref(),
        Some("Keyboard missing")
    );

    // Carol in SQLite
    let rep_carol = &reps[2];
    assert_eq!(rep_carol.name, "Carol");
    assert_eq!(rep_carol.ticket_count, Some(0));
    assert_eq!(rep_carol.open_ticket_count, Some(0));
    assert_eq!(rep_carol.has_tickets, Some(false));
    assert_eq!(rep_carol.first_ticket_subject, None);

    // Filter in SQLite WHERE clause: ticket_count >= 2
    let filtered_reps = desk
        .representatives()
        .aggregate(r::ticket_count)
        .filter(r::ticket_count.gte(2_i64))
        .all()
        .await
        .unwrap();

    assert_eq!(filtered_reps.len(), 1);
    assert_eq!(filtered_reps[0].name, "Alice");
    assert_eq!(filtered_reps[0].ticket_count, Some(2));
}

#[tokio::test]
async fn sum_and_filtered_aggregates_on_orders() {
    let mem = ash_memory::Memory::new();
    let ctx = ash_core::Context::new(mem);

    let order = Order::create(&ctx)
        .customer_name("Will")
        .await
        .unwrap();

    // Add line items
    LineItem::create(&ctx)
        .order_id(order.id)
        .sku("KEYBOARD-RGB")
        .amount(120)
        .status("paid")
        .await
        .unwrap();

    LineItem::create(&ctx)
        .order_id(order.id)
        .sku("MOUSE-WIRELESS")
        .amount(80)
        .status("paid")
        .await
        .unwrap();

    LineItem::create(&ctx)
        .order_id(order.id)
        .sku("MONITOR-ARM")
        .amount(150)
        .status("pending")
        .await
        .unwrap();

    // Query order with sum and filtered aggregates
    let loaded = Order::query(&ctx)
        .aggregate("item_count")
        .aggregate("total_amount")
        .aggregate("paid_amount")
        .aggregate("has_items")
        .aggregate("pending_item_sku")
        .one()
        .await
        .unwrap();

    assert_eq!(loaded.item_count, Some(3));
    assert_eq!(loaded.total_amount, Some(350)); // 120 + 80 + 150
    assert_eq!(loaded.paid_amount, Some(200));  // 120 + 80
    assert_eq!(loaded.has_items, Some(true));
    assert_eq!(loaded.pending_item_sku.as_deref(), Some("MONITOR-ARM"));
}
