use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use ash_core::{ActionKind, Context, resource};
use ash_memory::Memory;
use ash_pubsub::{PubSub, PubSubNotifier, topic_matches};

resource! {
    Order {
        table "orders";

    attributes {
        id: Uuid [pk];
        customer: String;
        amount: i64;
        status: String;
    }

    actions {
        create create {
            primary;
            accept [customer, amount];
            change set_attribute(status, "pending");
        }

        read read {
            primary;
        }

        update complete {
            change set_attribute(status, "completed");
        }
    }
    }}

#[test]
fn test_topic_pattern_matching_rules() {
    // Global wildcard
    assert!(topic_matches("*", "orders:create"));
    assert!(topic_matches("*", "any:topic:at:all"));

    // Exact matches
    assert!(topic_matches("orders:create", "orders:create"));
    assert!(!topic_matches("orders:create", "orders:paid"));

    // Single segment wildcard
    assert!(topic_matches("orders:*:paid", "orders:123:paid"));
    assert!(!topic_matches("orders:*:paid", "orders:123:cancelled"));
    assert!(!topic_matches("orders:*:paid", "orders:123:456:paid"));

    // Trailing wildcard matches all remaining segments
    assert!(topic_matches("orders:*", "orders:create"));
    assert!(topic_matches("orders:*", "orders:123:paid"));
    assert!(topic_matches("orders:*", "orders:123:status:changed"));
    assert!(!topic_matches("orders:*", "users:created"));
}

#[tokio::test]
async fn test_pubsub_topic_and_wildcard_subscriptions() {
    let pubsub = Arc::new(PubSub::new());
    let notifier = Arc::new(PubSubNotifier::new(Arc::clone(&pubsub)));
    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    // Subscribe with wildcard: "order:*"
    let mut sub_all = pubsub.subscribe("order:*");
    // Subscribe with exact: "order:create"
    let mut sub_create = pubsub.subscribe("order:create");

    let order = Order::create(&ctx)
        .customer("Bob")
        .amount(350)
        .await
        .expect("create order");

    // sub_all receives create
    let notif_all = tokio::time::timeout(Duration::from_millis(100), sub_all.recv())
        .await
        .expect("timed out")
        .expect("received notification");
    assert_eq!(notif_all.action, "create");
    assert_eq!(notif_all.id, order.id);
    assert_eq!(notif_all.action_kind, ActionKind::Create);

    // sub_create receives create
    let notif_create = tokio::time::timeout(Duration::from_millis(100), sub_create.recv())
        .await
        .expect("timed out")
        .expect("received notification");
    assert_eq!(notif_create.id, order.id);

    // Now update order
    let _ = order.complete(&ctx).await.expect("complete order");

    // sub_all receives complete
    let notif_complete = tokio::time::timeout(Duration::from_millis(100), sub_all.recv())
        .await
        .expect("timed out")
        .expect("received notification");
    assert_eq!(notif_complete.action, "complete");
    assert_eq!(notif_complete.action_kind, ActionKind::Update);

    // sub_create does NOT receive complete
    let sub_create_check = tokio::time::timeout(Duration::from_millis(50), sub_create.recv()).await;
    assert!(sub_create_check.is_err(), "sub_create should not receive complete event");
}

#[tokio::test]
async fn test_pubsub_deduplicated_multi_topic_delivery() {
    let pubsub = Arc::new(PubSub::new());
    // Wildcard subscriber that matches both "order:create" and "order:<id>:create"
    let mut sub_all = pubsub.subscribe("order:*");

    let notifier = Arc::new(PubSubNotifier::new(Arc::clone(&pubsub)));
    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    let _order = Order::create(&ctx)
        .customer("Charlie")
        .amount(500)
        .await
        .expect("create order");

    // Must receive exactly ONE notification for this create, not two!
    let first = tokio::time::timeout(Duration::from_millis(100), sub_all.recv())
        .await
        .expect("timed out")
        .expect("first notification");
    assert_eq!(first.action, "create");

    // A second notification should NOT arrive
    let second_check = tokio::time::timeout(Duration::from_millis(50), sub_all.recv()).await;
    assert!(
        second_check.is_err(),
        "Subscription matching multiple published topics must only receive one event!"
    );
}

#[tokio::test]
async fn test_pubsub_custom_topic_formatting() {
    let pubsub = Arc::new(PubSub::new());
    // Custom topic formatter: "events.Order.create"
    let notifier = Arc::new(
        PubSubNotifier::new(Arc::clone(&pubsub)).with_topics(|notif| {
            vec![format!("events.{}.{}", notif.resource, notif.action)]
        }),
    );
    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    let mut sub = pubsub.subscribe("events.Order.create");

    let order = Order::create(&ctx)
        .customer("Diana")
        .amount(750)
        .await
        .expect("create order");

    let notif = tokio::time::timeout(Duration::from_millis(100), sub.recv())
        .await
        .expect("timed out")
        .expect("received notification");
    assert_eq!(notif.id, order.id);
}
