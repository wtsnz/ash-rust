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
        .await
        .unwrap();
    drop(ctx);

    let ctx = open_sqlite(&db).await.unwrap();
    let as_customer = ctx.with_actor(actor_customer(customer));
    let listed = Ticket::query(&as_customer).load().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, opened.id);
    assert_eq!(listed[0].subject, "Printer is jammed");
}
