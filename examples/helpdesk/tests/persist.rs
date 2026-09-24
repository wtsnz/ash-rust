use ash_core::{Binary, Date, Float};
use helpdesk::{Ticket, actor_customer, open_sqlite};
use uuid::Uuid;

#[tokio::test]
async fn tickets_survive_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("helpdesk.db");
    let customer = Uuid::new_v4();

    let ctx = open_sqlite(&db).await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(customer));
    let opened = Ticket::open(&as_customer)
        .subject("Printer is jammed")
        .estimate(Float::parse("1.5").unwrap())
        .due_on(Date::parse("2024-02-29").unwrap())
        .attachment(Binary::from_bytes(b"hello".to_vec()))
        .await
        .unwrap();
    drop(ctx);

    let ctx = open_sqlite(&db).await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(customer));
    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, opened.id);
    assert_eq!(listed[0].subject, "Printer is jammed");
    assert_eq!(listed[0].estimate.as_ref().map(Float::as_str), Some("1.5"));
    assert_eq!(listed[0].due_on.as_ref().map(Date::as_str), Some("2024-02-29"));
    assert_eq!(
        listed[0].attachment.as_ref().map(Binary::as_bytes),
        Some(b"hello".as_slice())
    );
}
