use ash_core::Context;
use ash_memory::Memory;
use helpdesk::ticket::{fields as t, intake_store};
use helpdesk::{
    Representative, Status, Ticket, TicketActions, actor_customer, actor_representative,
};
use uuid::Uuid;

fn ctx() -> Context<Memory> {
    Context::new(Memory::new())
}

#[tokio::test]
async fn direct_instance_methods_and_into_future() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let as_alice = ctx.with_actor(actor_representative(alice.id));

    // 1. Create with builder and IntoFuture (.await without .call())
    let ticket = Ticket::open(&as_customer)
        .subject("Monitor flickering")
        .await
        .unwrap();
    assert_eq!(ticket.subject, "Monitor flickering");
    assert_eq!(ticket.status, Status::Open);

    // 2. Direct instance update
    let ticket = ticket
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();
    assert_eq!(ticket.representative_id, Some(alice.id));

    // 3. Zero-argument instance update
    let closed = ticket.close(&as_alice).await.unwrap();
    assert_eq!(closed.status, Status::Closed);
}

#[tokio::test]
async fn static_update_by_id() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let as_alice = ctx.with_actor(actor_representative(alice.id));

    let ticket = Ticket::open(&as_customer)
        .subject("Keyboard issue")
        .call()
        .await
        .unwrap();

    // Calling by ID statically
    let ticket = Ticket::assign(&as_alice, ticket.id)
        .representative_id(alice.id)
        .call()
        .await
        .unwrap();
    assert_eq!(ticket.representative_id, Some(alice.id));

    let ticket = Ticket::close(&as_alice, ticket.id)
        .call()
        .await
        .unwrap();
    assert_eq!(ticket.status, Status::Closed);
}

#[tokio::test]
async fn fluent_filter_composition_and_rel_accessors() {
    let ctx = ctx();
    let alice = Representative::create(&ctx).name("Alice").await.unwrap();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));
    let as_alice = ctx.with_actor(actor_representative(alice.id));

    let ticket1 = Ticket::open(&as_customer)
        .subject("Network down")
        .await
        .unwrap();
    let _ticket2 = Ticket::open(&as_customer)
        .subject("Printer broken")
        .await
        .unwrap();
    let ticket3 = Ticket::open(&as_customer)
        .subject("Slow laptop")
        .await
        .unwrap();

    ticket1
        .assign(&as_alice)
        .representative_id(alice.id)
        .await
        .unwrap();
    ticket3.close(&as_customer).await.unwrap();

    // Filter composition using overloaded operators (&, |, !)
    let open_condition = t::status.eq("open");
    let printer_condition = t::subject.eq("Printer broken");
    let network_condition = t::subject.eq("Network down");

    let compound_filter = open_condition & (printer_condition | network_condition);

    let results = Ticket::query(&as_customer)
        .filter(compound_filter)
        .include(t::representative)
        .sort(t::subject)
        .load()
        .await
        .unwrap();

    assert_eq!(results.len(), 2);
    assert_eq!(results[0].subject, "Network down");
    assert_eq!(results[1].subject, "Printer broken");

    // Rel accessors: as_option() and expect_loaded()
    let rep_opt = results[0].representative.as_option().unwrap();
    assert_eq!(rep_opt.unwrap().name, "Alice");
    assert_eq!(
        results[0].representative.expect_loaded("must be loaded").as_ref().unwrap().name,
        "Alice"
    );

    let unassigned_opt = results[1].representative.as_option().unwrap();
    assert_eq!(unassigned_opt, None);

    // Rel<Vec<T>>::as_slice() on has_many
    let rep_with_tickets = Representative::query(&as_alice)
        .include("tickets")
        .one()
        .await
        .unwrap();
    let tickets_slice = rep_with_tickets.tickets.as_slice().unwrap();
    assert_eq!(tickets_slice.len(), 1);
    assert_eq!(tickets_slice[0].id, ticket1.id);
}

#[tokio::test]
async fn generic_action_and_manual_intake() {
    let ctx = ctx();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));

    // Generic action builder
    let analysis = Ticket::analyze_subject(&as_customer)
        .text("Help urgent crisis!")
        .await
        .unwrap();
    assert_eq!(analysis.word_count, 3);
    assert!(analysis.urgent);

    // Manual intake with custom persist closure
    let manual = Ticket::intake(&as_customer)
        .subject("Intake via webhook")
        .persist(|_ctx, ticket| async move {
            intake_store()
                .lock()
                .unwrap()
                .insert(ticket.id, ticket.clone());
            Ok(ticket)
        })
        .await
        .unwrap();
    assert_eq!(manual.subject, "Intake via webhook");
}

#[tokio::test]
async fn changeset_introspection_before_commit() {
    let ctx = ctx();
    let customer_id = Uuid::new_v4();
    let as_customer = ctx.with_actor(actor_customer(customer_id));

    // Changeset can be inspected before committing
    let changeset = Ticket::open(&as_customer)
        .subject("Draft ticket")
        .changeset()
        .unwrap();

    assert_eq!(
        changeset.attributes().get("subject"),
        Some(&ash_core::Value::String("Draft ticket".into()))
    );
    assert_eq!(
        changeset.attributes().get("status"),
        Some(&ash_core::Value::String("open".into()))
    );

    let committed = changeset.commit(&as_customer).await.unwrap();
    assert_eq!(committed.subject, "Draft ticket");
}
