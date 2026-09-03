use ash_core::{Context, Error, Filter, Rel, Value};
use ash_memory::Memory;
use helpdesk::ticket::{fields, intake_store};
use helpdesk::{
    Representative, Status, Ticket, TicketActions, actor_customer, actor_representative,
    intake_get,
};
use uuid::Uuid;

fn ctx() -> Context<Memory> {
    Context::new(Memory::new())
}

#[tokio::test]
async fn customer_opens_and_reads_own_ticket() {
    let ctx = ctx();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));

    let ticket = Ticket::open(&as_customer)
        .subject("Broken mouse")
        .await
        .unwrap();
    assert_eq!(ticket.status, Status::Open);
    assert_eq!(ticket.opener_id, customer_id);
    assert_eq!(ticket.representative_id, None);

    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, ticket.id);
}

#[tokio::test]
async fn other_customer_does_not_see_ticket() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_other = ctx.with_actor(actor_customer(Uuid::new_v4()));

    Ticket::open(&as_customer)
        .subject("Broken mouse")
        .await
        .unwrap();
    let listed = Ticket::query(&as_other).load().await.unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn representative_sees_unassigned_then_only_assigned() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let bob = Representative::create(&ctx).name("Bob").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let as_bob = ctx.with_actor(actor_representative(bob.id));

    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    assert_eq!(Ticket::query(&as_bob).load().await.unwrap().len(), 1);

    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let bob_sees = Ticket::query(&as_bob).load().await.unwrap();
    assert!(bob_sees.is_empty());

    let alice_sees = Ticket::query(&as_alice).load().await.unwrap();
    assert_eq!(alice_sees.len(), 1);
    assert_eq!(alice_sees[0].representative_id, Some(alice.id));
}

#[tokio::test]
async fn opener_still_sees_ticket_after_assign() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let as_alice = ctx.with_actor(actor_representative(alice.id));

    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed.len(), 1);
}

#[tokio::test]
async fn opener_and_assignee_can_close() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));

    let first = Ticket::open(&as_customer).subject("One").await.unwrap();
    let closed = first.close(&as_customer).await.unwrap();
    assert_eq!(closed.status, Status::Closed);

    let second = Ticket::open(&as_customer).subject("Two").await.unwrap();
    let assigned = second
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();
    let closed = assigned.close(&as_alice).await.unwrap();
    assert_eq!(closed.status, Status::Closed);
}

#[tokio::test]
async fn other_representative_cannot_close_assigned_ticket() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let bob = Representative::create(&ctx).name("Bob").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let as_bob = ctx.with_actor(actor_representative(bob.id));

    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    let assigned = ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    // Calling by ID fails with NotFound because Bob's read policy hides assigned tickets
    let err = Ticket::close(&as_bob, assigned.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound));

    // Calling on an existing record directly fails with Forbidden because Bob's write policy rejects him
    let err = assigned.close(&as_bob).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden));
}

#[tokio::test]
async fn customer_cannot_assign() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));

    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    let err = ticket
        .assign(&as_customer)
        .representative_id(alice.id)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden));
}

#[tokio::test]
async fn open_without_actor_is_forbidden() {
    let ctx = ctx();
    let err = Ticket::open(&ctx)
        .subject("Printer")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden));
}

#[tokio::test]
async fn user_filter_and_policy_filter_combine() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("open one").await.unwrap();
    let closed = Ticket::open(&as_customer).subject("close me").await.unwrap();
    closed.close(&as_customer).await.unwrap();

    // Uses operator overloading (&) for filter composition
    let open_only = Ticket::query(&as_customer)
        .filter(Filter::eq("status", "open") & Filter::ne("status", "closed"))
        .load()
        .await
        .unwrap();
    assert_eq!(open_only.len(), 1);
    assert_eq!(open_only[0].subject, "open one");
}

#[tokio::test]
async fn extra_input_is_rejected() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let err = ash_core::create::<Ticket, _>(
        &as_customer,
        "open",
        ash_core::fields! {
            "subject" => "hi",
            "status" => "closed",
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::NotAccepted { .. }));
}

#[tokio::test]
async fn representatives_can_be_listed() {
    let ctx = ctx();
    Representative::create(&ctx).name("Alice").await.unwrap();
    Representative::create(&ctx).name("Bob").await.unwrap();
    let people = Representative::query(&ctx).load().await.unwrap();
    assert_eq!(people.len(), 2);
}

#[tokio::test]
async fn generic_action_analyzes_text() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let analysis = Ticket::analyze_subject(&as_customer)
        .text("Printer is jammed!")
        .await
        .unwrap();
    assert_eq!(analysis.word_count, 3);
    assert!(analysis.urgent);
}

#[tokio::test]
async fn generic_action_requires_actor() {
    let ctx = ctx();
    let err = Ticket::analyze_subject(&ctx)
        .text("hi")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden));
}

#[tokio::test]
async fn manual_intake_skips_data_layer() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let ticket = Ticket::intake(&as_customer)
        .subject("From email")
        .persist(|_ctx, ticket| async move {
            intake_store()
                .lock()
                .expect("intake store")
                .insert(ticket.id, ticket.clone());
            Ok(ticket)
        })
        .await
        .unwrap();
    assert_eq!(ticket.status, Status::Open);
    assert!(Ticket::query(&as_customer).load().await.unwrap().is_empty());
    let stored = intake_get(ticket.id).expect("intake store");
    assert_eq!(stored.subject, "From email");
    assert_eq!(stored.opener_id, as_customer.actor.as_ref().unwrap().id);
}

#[tokio::test]
async fn changeset_applies_changes_before_commit() {
    let ctx = ctx();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let changeset = Ticket::open(&as_customer)
        .subject("Inspect me")
        .changeset()
        .unwrap();
    assert_eq!(
        changeset.attributes().get("status"),
        Some(&Value::String("open".into()))
    );
    assert_eq!(
        changeset.attributes().get("opener_id"),
        Some(&Value::Uuid(customer_id))
    );
    let ticket = changeset.commit(&as_customer).await.unwrap();
    assert_eq!(ticket.subject, "Inspect me");
}

#[tokio::test]
async fn query_sorts_by_subject() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("zeta").await.unwrap();
    Ticket::open(&as_customer).subject("alpha").await.unwrap();
    let listed = Ticket::query(&as_customer)
        .sort("subject")
        .load()
        .await
        .unwrap();
    assert_eq!(listed[0].subject, "alpha");
    assert_eq!(listed[1].subject, "zeta");
}

#[tokio::test]
async fn create_rejects_manual_action() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let err = ash_core::create::<Ticket, _>(
        &as_customer,
        "intake",
        ash_core::fields! { "subject" => "nope" },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::ManualRequired { .. }));
}

#[tokio::test]
async fn calculation_is_absent_until_loaded() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let ticket = Ticket::open(&as_customer).subject("hello world").await.unwrap();
    assert_eq!(ticket.subject_length, None);

    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed[0].subject_length, None);

    let loaded = Ticket::query(&as_customer)
        .calc(fields::subject_length)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.subject_length, Some(11));
}

#[tokio::test]
async fn calculation_filter_runs_without_loading() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("hi").await.unwrap();
    Ticket::open(&as_customer).subject("hello world").await.unwrap();

    let long = Ticket::query(&as_customer)
        .filter(fields::subject_length.gt(5_i64))
        .load()
        .await
        .unwrap();
    assert_eq!(long.len(), 1);
    assert_eq!(long[0].subject, "hello world");
    assert_eq!(long[0].subject_length, None);
}

#[tokio::test]
async fn belongs_to_is_not_loaded_until_asked() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let listed = Ticket::query(&as_customer).one().await.unwrap();
    assert!(matches!(listed.representative, Rel::NotLoaded));

    let loaded = Ticket::query(&as_customer)
        .include(fields::representative)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.representative.as_option().unwrap().unwrap().name, "Alice");
}

#[tokio::test]
async fn belongs_to_null_fk_loads_none() {
    let ctx = ctx();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("Printer").await.unwrap();
    let loaded = Ticket::query(&as_customer)
        .include("representative")
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.representative.as_option().unwrap(), None);
}

#[tokio::test]
async fn has_many_loads_assigned_tickets() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let listed = Representative::query(&as_alice).one().await.unwrap();
    assert!(matches!(listed.tickets, Rel::NotLoaded));

    let loaded = Representative::query(&as_alice)
        .include("tickets")
        .one()
        .await
        .unwrap();
    let tickets = loaded.tickets.as_slice().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].id, ticket.id);
    assert!(matches!(tickets[0].representative, Rel::NotLoaded));
}

#[tokio::test]
async fn has_many_respects_ticket_read_policy() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_other = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let loaded = Representative::query(&as_other)
        .include("tickets")
        .one()
        .await
        .unwrap();
    assert!(loaded.tickets.as_slice().unwrap().is_empty());
}
