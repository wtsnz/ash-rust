use ash_core::AshEnum;
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use helpdesk::{
    HELPDESK_DEF, Helpdesk, Representative, Ticket, TicketActions, actor_customer,
    actor_representative,
};
use uuid::Uuid;

#[tokio::test]
async fn domain_metadata_and_validation() {
    assert_eq!(HELPDESK_DEF.name, "Helpdesk");
    assert!(HELPDESK_DEF.has_resource("Ticket"));
    assert!(HELPDESK_DEF.has_resource("Representative"));
    assert!(!HELPDESK_DEF.has_resource("NonExistent"));
    assert!(HELPDESK_DEF.validate().is_ok());
    assert_eq!(HELPDESK_DEF.resources.len(), 2);

    let desk = Helpdesk::new(Memory::new());
    assert_eq!(desk.def().name, "Helpdesk");
}

#[tokio::test]
async fn domain_code_interfaces_and_lifecycle() {
    let desk = Helpdesk::new(Memory::new());

    // 1. Code interface: create_representative
    let rep = desk.create_representative("Support Tech").await.unwrap();
    assert_eq!(rep.name, "Support Tech");

    // 2. Code interface: get_representative
    let fetched_rep = desk.get_representative(rep.id).await.unwrap();
    assert_eq!(fetched_rep.id, rep.id);

    // 3. Actors on domain
    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    let as_rep = desk.with_actor(actor_representative(rep.id));

    // 4. Code interface: open_ticket
    let ticket = as_customer
        .open_ticket("Cannot connect to VPN")
        .await
        .unwrap();
    assert_eq!(ticket.subject, "Cannot connect to VPN");
    assert_eq!(ticket.status.as_str(), "open");

    // 5. Code interface: get_ticket
    let fetched_ticket = as_customer.get_ticket(ticket.id).await.unwrap();
    assert_eq!(fetched_ticket.id, ticket.id);

    // 6. Code interface: assign_ticket (on record)
    let assigned = as_rep.assign_ticket(&ticket, rep.id).await.unwrap();
    assert_eq!(assigned.representative_id, Some(rep.id));

    // 7. Code interface: close_ticket (on record)
    let closed = as_rep.close_ticket(&assigned).await.unwrap();
    assert_eq!(closed.status.as_str(), "closed");

    // 8. Code interface: list_tickets (returns Query)
    let all_tickets = as_customer.list_tickets().all().await.unwrap();
    assert_eq!(all_tickets.len(), 1);

    // 9. Query first() helper
    let first = as_customer.list_tickets().first().await.unwrap();
    assert!(first.is_some());
    assert_eq!(first.unwrap().id, ticket.id);
}

#[tokio::test]
async fn domain_deref_to_context() {
    let desk = Helpdesk::new(Memory::new());
    let rep = desk.create_representative("Bob").await.unwrap();

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));

    // &as_customer derefs to &Context<Memory>!
    let ticket = Ticket::open(&as_customer)
        .subject("Keyboard broken")
        .await
        .unwrap();

    let as_bob = desk.with_actor(actor_representative(rep.id));
    let assigned = ticket.assign(&as_bob).representative_id(rep.id).await.unwrap();
    assert_eq!(assigned.representative_id, Some(rep.id));
}

#[tokio::test]
async fn domain_default_resource_queries() {
    let desk = Helpdesk::new(Memory::new());
    desk.create_representative("Alice").await.unwrap();
    desk.create_representative("Bob").await.unwrap();

    // Default plural accessors generated on domain: .representatives() and .tickets()
    let reps = desk.representatives().all().await.unwrap();
    assert_eq!(reps.len(), 2);

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    as_customer.open_ticket("First").await.unwrap();
    as_customer.open_ticket("Second").await.unwrap();

    let tickets = as_customer.tickets().all().await.unwrap();
    assert_eq!(tickets.len(), 2);
}

#[tokio::test]
async fn domain_generic_methods() {
    let desk = Helpdesk::new(Memory::new());
    let rep = desk.create_representative("Charlie").await.unwrap();

    // Generic get on domain
    let rep_again = desk.get::<Representative>(rep.id).await.unwrap();
    assert_eq!(rep_again.name, "Charlie");

    // Generic query on domain
    let reps = desk.query::<Representative>().all().await.unwrap();
    assert_eq!(reps.len(), 1);
}

#[tokio::test]
async fn domain_install_with_schema_support() {
    let sqlite = Sqlite::memory().await.unwrap();
    let desk = Helpdesk::new(sqlite);

    // Installs all domain resources at once via SchemaSupport
    desk.install().await.unwrap();

    let rep = desk.create_representative("Dave").await.unwrap();
    assert_eq!(rep.name, "Dave");

    let customer_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    let ticket = as_customer.open_ticket("Screen flickers").await.unwrap();
    assert_eq!(ticket.subject, "Screen flickers");
}
