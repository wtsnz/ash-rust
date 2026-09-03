use ash_core::{Error, Multi};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use helpdesk::representative::Representative;
use helpdesk::ticket::{Ticket, actor_representative};
use helpdesk::{Helpdesk, actor_customer};
use uuid::Uuid;

#[tokio::test]
async fn test_multi_memory_success() {
    let mem = Memory::new();
    let desk = Helpdesk::new(mem);

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    // Define a Multi pipeline:
    // 1. Create representative "Alice"
    // 2. Insert metadata "priority"
    // 3. Dynamically create ticket using Representative's presence
    // 4. Custom async run step to assign the ticket
    let rep_cs = Representative::create(&as_customer)
        .name("Alice")
        .changeset()
        .unwrap();

    let multi = Multi::new()
        .create("rep", rep_cs)
        .insert("priority", 42_i64)
        .create_from("ticket", move |ctx, _results| {
            Ticket::open(ctx)
                .subject("Network down in room 3")
                .changeset()
        })
        .run("assign", move |ctx, results| async move {
            let rep = results.get::<Representative>("rep")?;
            let ticket = results.get::<Ticket>("ticket")?;
            let as_rep = ctx.with_actor(actor_representative(rep.id));
            ticket
                .assign_on(&as_rep)
                .representative_id(rep.id)
                .call()
                .await
        });

    let results = multi.commit(&as_customer).await.unwrap();

    // Verify all steps produced expected types and values
    assert_eq!(results.len(), 4);
    assert!(results.contains("rep"));
    assert!(results.contains("priority"));
    assert!(results.contains("ticket"));
    assert!(results.contains("assign"));

    let rep: &Representative = results.get("rep").unwrap();
    assert_eq!(rep.name, "Alice");

    let priority: &i64 = results.get("priority").unwrap();
    assert_eq!(*priority, 42);

    let assigned_ticket: &Ticket = results.get("assign").unwrap();
    assert_eq!(assigned_ticket.representative_id, Some(rep.id));
    assert_eq!(assigned_ticket.subject, "Network down in room 3");

    // Verify records exist in data store
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].name, "Alice");

    let tickets = as_customer.list_tickets().all().await.unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].id, assigned_ticket.id);
}

#[tokio::test]
async fn test_multi_memory_rollback_on_validation_failure() {
    let mem = Memory::new();
    let desk = Helpdesk::new(mem);

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    let rep_cs = Representative::create(&as_customer)
        .name("Bob")
        .changeset()
        .unwrap();

    // Step 1: create Bob
    // Step 2: create Ticket with invalid subject (length < 2 fails validation)
    let multi = Multi::new()
        .create("rep", rep_cs)
        .create_from("ticket", move |ctx, _results| {
            Ticket::open(ctx).subject("X").changeset()
        });

    let err = multi.commit(&as_customer).await.unwrap_err();

    // Verify Error::Multi specifies the failed step and reason
    assert_eq!(err.multi_step(), Some("ticket"));
    match err.multi_source() {
        Some(Error::Validation { field, .. }) => assert_eq!(field, "subject"),
        other => panic!("expected Error::Validation, got {:?}", other),
    }

    // CRITICAL: Bob must have been rolled back and not exist in memory!
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert_eq!(
        reps.len(),
        0,
        "Representative creation must be rolled back on multi failure"
    );
}

#[tokio::test]
async fn test_multi_memory_rollback_on_custom_step_error() {
    let mem = Memory::new();
    let desk = Helpdesk::new(mem);

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    let rep_cs = Representative::create(&as_customer)
        .name("Charlie")
        .changeset()
        .unwrap();

    let multi = Multi::new()
        .create("rep", rep_cs)
        .run("payment", |_ctx, _results| async move {
            Err::<(), _>(Error::Invalid("credit card declined".into()))
        });

    let err = multi.commit(&as_customer).await.unwrap_err();
    assert_eq!(err.multi_step(), Some("payment"));

    // Charlie must not exist!
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert!(reps.is_empty(), "Charlie must be rolled back");
}

#[tokio::test]
async fn test_multi_sqlite_success_and_domain_run_multi() {
    let db = Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(db);
    desk.install().await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    let rep_cs = Representative::create(&as_customer)
        .name("Dana")
        .changeset()
        .unwrap();

    let multi = Multi::new()
        .create("rep", rep_cs)
        .create_from("ticket", move |ctx, _results| {
            Ticket::open(ctx)
                .subject("Software license request")
                .changeset()
        });

    // Run using desk.run_multi
    let results = as_customer.run_multi(multi).await.unwrap();
    let rep: &Representative = results.get("rep").unwrap();
    let ticket: &Ticket = results.get("ticket").unwrap();

    assert_eq!(rep.name, "Dana");
    assert_eq!(ticket.subject, "Software license request");

    // Verify both rows exist in SQLite
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert_eq!(reps.len(), 1);
    assert_eq!(reps[0].name, "Dana");

    let tickets = as_customer.list_tickets().all().await.unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].id, ticket.id);
}

#[tokio::test]
async fn test_multi_sqlite_rollback_on_failure() {
    let db = Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(db);
    desk.install().await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    let rep_cs = Representative::create(&as_customer)
        .name("Eve")
        .changeset()
        .unwrap();

    let multi = Multi::new()
        .create("rep", rep_cs)
        .run("bomb", |_ctx, _results| async move {
            Err::<(), _>(Error::Invalid("database constraint or network error".into()))
        });

    let err = as_customer.run_multi(multi).await.unwrap_err();
    assert_eq!(err.multi_step(), Some("bomb"));

    // Verify Eve was rolled back in SQLite
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert!(
        reps.is_empty(),
        "Eve should not exist in SQLite after rollback"
    );
}

#[tokio::test]
async fn test_domain_transaction_and_nested_savepoints_in_sqlite() {
    let db = Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(db);
    desk.install().await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    // Outer transaction via desk.transaction
    as_customer
        .transaction(|tx_desk| async move {
            // 1. Create Frank in outer transaction
            let frank = tx_desk.create_representative("Frank").await.unwrap();
            assert_eq!(frank.name, "Frank");

            // 2. Run an inner Multi that fails (simulating a nested transaction with savepoint)
            let bad_multi = Multi::new()
                .create(
                    "rep_bad",
                    Representative::create(&tx_desk)
                        .name("BadRep")
                        .changeset()
                        .unwrap(),
                )
                .run("explode", |_ctx, _results| async move {
                    Err::<(), _>(Error::Invalid("fail inner".into()))
                });

            let inner_err = tx_desk.run_multi(bad_multi).await.unwrap_err();
            assert_eq!(inner_err.multi_step(), Some("explode"));

            // 3. Run an inner Multi that succeeds
            let good_multi = Multi::new().create(
                "rep_good",
                Representative::create(&tx_desk)
                    .name("GoodRep")
                    .changeset()
                    .unwrap(),
            );
            tx_desk.run_multi(good_multi).await.unwrap();

            Ok(())
        })
        .await
        .unwrap();

    // After outer transaction commits:
    // Frank and GoodRep should exist, BadRep must NOT exist!
    let reps = as_customer.list_representatives().all().await.unwrap();
    let names: Vec<_> = reps.iter().map(|r| r.name.as_str()).collect();
    assert!(names.contains(&"Frank"));
    assert!(names.contains(&"GoodRep"));
    assert!(!names.contains(&"BadRep"));
}

#[tokio::test]
async fn test_domain_transaction_rollback_when_closure_returns_err() {
    let db = Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(db);
    desk.install().await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    let res: ash_core::Result<()> = as_customer
        .transaction(|tx_desk| async move {
            tx_desk.create_representative("Grace").await.unwrap();
            // Outer transaction fails
            Err(Error::Invalid("abort outer transaction".into()))
        })
        .await;

    assert!(res.is_err());

    // Grace must not exist in SQLite
    let reps = as_customer.list_representatives().all().await.unwrap();
    assert!(reps.is_empty(), "Grace must be rolled back on outer error");
}
