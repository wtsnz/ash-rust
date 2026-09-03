pub mod cli;
pub mod representative;
pub mod ticket;

pub use representative::{REPRESENTATIVE_DEF, Representative};
pub use ticket::{
    Analysis, Status, TICKET_DEF, Ticket, TicketActions, actor_customer, actor_representative,
    intake_get,
};

use std::path::Path;

use ash_core::{DomainDef, domain};
use ash_sqlite::Sqlite;

domain! {
    domain Helpdesk;
    resources {
        Ticket {
            define open_ticket, action: open, args: [subject: String];
            define close_ticket, action: close, on: record;
            define assign_ticket, action: assign, on: record, args: [representative_id: ::uuid::Uuid];
            define get_ticket, action: read, get_by: id;
            define list_tickets, action: read;
        },
        Representative {
            define create_representative, action: create, args: [name: String];
            define list_representatives, action: read;
            define get_representative, action: read, get_by: id;
        },
    }
}

pub const HELPDESK: DomainDef = HELPDESK_DEF;

pub async fn open_sqlite(path: impl AsRef<Path>) -> ash_core::Result<Helpdesk<Sqlite>> {
    if let Some(parent) = path.as_ref().parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|err| {
            ash_core::Error::DataLayer(format!("create database directory: {err}"))
        })?;
    }
    let db = Sqlite::file(path).await?;
    let desk = Helpdesk::new(db);
    desk.install().await?;
    Ok(desk)
}

pub async fn demo() -> ash_core::Result<()> {
    use ash_memory::Memory;
    use uuid::Uuid;

    let desk = Helpdesk::new(Memory::new());
    let alice = desk.create_representative("Alice").await?;
    let bob = desk.create_representative("Bob").await?;

    let customer_id = Uuid::new_v4();
    let other_id = Uuid::new_v4();
    let as_customer = desk.with_actor(actor_customer(customer_id));
    let as_other = desk.with_actor(actor_customer(other_id));
    let as_alice = desk.with_actor(actor_representative(alice.id));
    let as_bob = desk.with_actor(actor_representative(bob.id));

    println!("Alice and Bob join the desk.");
    println!("Customer opens a ticket.");
    let ticket = as_customer.open_ticket("Printer is jammed").await?;
    println!("  {ticket}");

    println!("\nAnother customer lists tickets:");
    print_tickets(as_other.list_tickets().load().await?);

    println!("Bob lists the unassigned queue:");
    print_tickets(as_bob.list_tickets().load().await?);

    println!("Alice assigns it to herself.");
    let ticket = as_alice
        .assign_ticket(&ticket, alice.id)
        .await?;
    println!("  {ticket}");

    println!("\nBob's queue after assign:");
    print_tickets(as_bob.list_tickets().load().await?);

    println!("Alice sees it as assignee:");
    print_tickets(as_alice.list_tickets().load().await?);

    println!("The opener still sees it:");
    print_tickets(as_customer.list_tickets().load().await?);

    println!("Alice closes it.");
    let ticket = as_alice.close_ticket(&ticket).await?;
    println!("  {ticket}");

    let analysis = Ticket::analyze_subject(&as_customer)
        .text("Printer is jammed!")
        .await?;
    println!(
        "\nGeneric analyze_subject: {} words, urgent={}",
        analysis.word_count, analysis.urgent
    );

    let inbound = Ticket::intake(&as_customer)
        .subject("From the mailbox")
        .persist(|_ctx, ticket| async move {
            ticket::intake_store()
                .lock()
                .expect("intake store")
                .insert(ticket.id, ticket.clone());
            Ok(ticket)
        })
        .await?;
    println!("Manual intake (not in Memory): {inbound}");

    Ok(())
}

fn print_tickets(tickets: Vec<Ticket>) {
    if tickets.is_empty() {
        println!("  (none)");
        return;
    }
    for ticket in tickets {
        println!("  {ticket}");
    }
}
