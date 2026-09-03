use ash_core::{Changeset, Context, Error, Filter, Value};
use ash_sqlite::Sqlite;
use helpdesk::{
    REPRESENTATIVE_DEF, Representative, Status, TICKET_DEF, Ticket, TicketActions, actor_customer,
    actor_representative,
};
use uuid::Uuid;

async fn ctx() -> Context<Sqlite> {
    let db = Sqlite::memory().await.unwrap();
    db.install(&[&TICKET_DEF, &REPRESENTATIVE_DEF])
        .await
        .unwrap();
    Context::new(db)
}

#[tokio::test]
async fn customer_opens_and_reads_own_ticket() {
    let ctx = ctx().await;
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));

    let ticket = Ticket::open(&as_customer)
        .subject("Broken mouse")
        .await
        .unwrap();
    assert_eq!(ticket.status, Status::Open);
    assert_eq!(ticket.opener_id, customer_id);

    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, ticket.id);
}

#[tokio::test]
async fn other_customer_does_not_see_ticket() {
    let ctx = ctx().await;
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_other = ctx.with_actor(actor_customer(Uuid::new_v4()));

    Ticket::open(&as_customer).subject("Broken mouse").await.unwrap();
    let listed = Ticket::query(&as_other).load().await.unwrap();
    assert!(listed.is_empty());
}

#[tokio::test]
async fn representative_sees_unassigned_then_only_assigned() {
    let ctx = ctx().await;
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
async fn user_filter_and_policy_filter_combine() {
    let ctx = ctx().await;
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("open one").await.unwrap();
    let closed = Ticket::open(&as_customer).subject("close me").await.unwrap();
    closed.close(&as_customer).await.unwrap();

    let open_only = Ticket::query(&as_customer)
        .filter(Filter::eq("status", "open"))
        .load()
        .await
        .unwrap();
    assert_eq!(open_only.len(), 1);
    assert_eq!(open_only[0].subject, "open one");
}

#[tokio::test]
async fn query_sorts_by_subject() {
    let ctx = ctx().await;
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
async fn changeset_round_trips_through_sqlite() {
    let ctx = ctx().await;
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let changeset = Changeset::<Ticket>::for_create(
        &as_customer,
        "open",
        ash_core::fields! { "subject" => "Inspect me" },
    )
    .unwrap();
    assert_eq!(
        changeset.attributes().get("status"),
        Some(&Value::String("open".into()))
    );
    let ticket = changeset.commit(&as_customer).await.unwrap();
    assert_eq!(ticket.subject, "Inspect me");

    let loaded = Ticket::query(&as_customer).one().await.unwrap();
    assert_eq!(loaded.id, ticket.id);
}

#[tokio::test]
async fn customer_cannot_assign() {
    let ctx = ctx().await;
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
async fn calculation_is_selected_in_sql() {
    let ctx = ctx().await;
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("hello world").await.unwrap();
    let loaded = Ticket::query(&as_customer)
        .calc("subject_length")
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.subject_length, Some(11));
}

#[tokio::test]
async fn calculation_filter_is_sql_where() {
    let ctx = ctx().await;
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    Ticket::open(&as_customer).subject("hi").await.unwrap();
    Ticket::open(&as_customer).subject("hello world").await.unwrap();

    let long = Ticket::query(&as_customer)
        .filter(Filter::gt("subject_length", 5_i64))
        .load()
        .await
        .unwrap();
    assert_eq!(long.len(), 1);
    assert_eq!(long[0].subject, "hello world");
    assert_eq!(long[0].subject_length, None);
}

#[tokio::test]
async fn belongs_to_loads_representative() {
    let ctx = ctx().await;
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let loaded = Ticket::query(&as_customer)
        .include("representative")
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.representative.as_option().unwrap().unwrap().name, "Alice");
}

#[tokio::test]
async fn has_many_loads_assigned_tickets() {
    let ctx = ctx().await;
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(Uuid::new_v4()));
    let as_alice = ctx.with_actor(actor_representative(alice.id));
    let ticket = Ticket::open(&as_customer).subject("Printer").await.unwrap();
    ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();

    let loaded = Representative::query(&as_alice)
        .include("tickets")
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.tickets.as_slice().unwrap()[0].id, ticket.id);
}
