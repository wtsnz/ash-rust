use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use ash_core::{
    ActionKind, Context, Multi, Notification, Resource, SchemaSupport, SyncFnNotifier, resource,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

static RESOURCE_NOTIFIER_CALLS: AtomicUsize = AtomicUsize::new(0);

const STATIC_NOTIFIER: SyncFnNotifier<fn(&Notification) -> ash_core::Result<()>> =
    SyncFnNotifier::new("static_counter", |_notif| {
        RESOURCE_NOTIFIER_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    });

resource! {
    Order {
        table "orders";

    attributes {
        id: Uuid [pk];
        customer: String;
        amount: i64;
        status: String;
    }

    notifiers [
        &STATIC_NOTIFIER
    ]

    actions {
        create create {
            primary;
            accept [customer, amount];
            change set_attribute(status, "pending");
            validate present(customer);
            validate numericality(amount, min: 1);
        }

        read read {
            primary;
        }

        update complete {
            change set_attribute(status, "completed");
        }

        destroy cancel {
            primary;
        }
    }
    }}

#[tokio::test]
async fn test_standalone_crud_notifications() {
    let received = Arc::new(Mutex::new(Vec::<Notification>::new()));
    let received_clone = Arc::clone(&received);

    let notifier = Arc::new(SyncFnNotifier::new("test_listener", move |notif| {
        received_clone.lock().unwrap().push(notif.clone());
        Ok(())
    }));

    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    // 1. Create action
    let order = Order::create(&ctx)
        .customer("Alice")
        .amount(120)
        .await
        .expect("create order");

    {
        let list = received.lock().unwrap();
        assert_eq!(list.len(), 1);
        let n = &list[0];
        assert_eq!(n.resource, "Order");
        assert_eq!(n.action, "create");
        assert_eq!(n.action_kind, ActionKind::Create);
        assert_eq!(n.id, order.id);
        assert_eq!(n.get("status").and_then(|v| v.as_str()), Some("pending"));
        assert_eq!(n.get("amount").and_then(|v| v.as_int()), Some(120));
        assert!(n.previous_fields.is_none());
    }

    // 2. Update action
    let completed = order.complete(&ctx).await.expect("complete order");

    {
        let list = received.lock().unwrap();
        assert_eq!(list.len(), 2);
        let n = &list[1];
        assert_eq!(n.action, "complete");
        assert_eq!(n.action_kind, ActionKind::Update);
        assert_eq!(n.get("status").and_then(|v| v.as_str()), Some("completed"));
        assert_eq!(n.previous("status").and_then(|v| v.as_str()), Some("pending"));
    }

    // 3. Destroy action
    ash_core::destroy::<Order, _>(&ctx, "cancel", completed.id)
        .await
        .expect("cancel order");

    {
        let list = received.lock().unwrap();
        assert_eq!(list.len(), 3);
        let n = &list[2];
        assert_eq!(n.action, "cancel");
        assert_eq!(n.action_kind, ActionKind::Destroy);
        assert_eq!(n.get("status").and_then(|v| v.as_str()), Some("completed"));
    }

    // 4. Verify static resource-level notifier was called at least 3 times
    assert!(RESOURCE_NOTIFIER_CALLS.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn test_multi_atomic_rollback_discards_notifications_in_memory() {
    let received = Arc::new(Mutex::new(Vec::<Notification>::new()));
    let received_clone = Arc::clone(&received);

    let notifier = Arc::new(SyncFnNotifier::new("test_listener", move |notif| {
        received_clone.lock().unwrap().push(notif.clone());
        Ok(())
    }));

    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    let cs1 = Order::create(&ctx).customer("Charlie").amount(100).changeset().unwrap();

    // Multi transaction with a failing step
    let multi = Multi::new()
        .create("order1", cs1)
        .run("fail_step", |_ctx, _results| async move {
            Err::<(), ash_core::Error>(ash_core::Error::Invalid("forced step failure".into()))
        });

    let res = ctx.run_multi(multi).await;
    assert!(res.is_err(), "multi must fail due to forced step failure");

    // Crucial check: zero notifications must be received because transaction rolled back!
    let list = received.lock().unwrap();
    assert_eq!(
        list.len(),
        0,
        "Notifications must not be emitted when Multi transaction fails and rolls back!"
    );
}

#[tokio::test]
async fn test_multi_success_dispatches_all_notifications_in_sqlite() {
    let sqlite = Sqlite::memory().await.expect("sqlite memory");
    sqlite.install_resources(&[&Order::DEF]).await.expect("install schema");

    let received = Arc::new(Mutex::new(Vec::<Notification>::new()));
    let received_clone = Arc::clone(&received);

    let notifier = Arc::new(SyncFnNotifier::new("test_listener", move |notif| {
        received_clone.lock().unwrap().push(notif.clone());
        Ok(())
    }));

    let ctx = Context::new(sqlite).with_notifier(notifier);

    let cs1 = Order::create(&ctx).customer("Elena").amount(200).changeset().unwrap();
    let cs2 = Order::create(&ctx).customer("Frank").amount(400).changeset().unwrap();

    // Multi pipeline that succeeds completely
    let multi = Multi::new()
        .create("order1", cs1)
        .create("order2", cs2);

    let res = ctx.run_multi(multi).await.expect("multi succeeds");
    let o1: &Order = res.get("order1").unwrap();
    let o2: &Order = res.get("order2").unwrap();

    // Verify all notifications arrived in order
    let list = received.lock().unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].id, o1.id);
    assert_eq!(list[0].get("customer").and_then(|v| v.as_str()), Some("Elena"));
    assert_eq!(list[1].id, o2.id);
    assert_eq!(list[1].get("customer").and_then(|v| v.as_str()), Some("Frank"));
}
