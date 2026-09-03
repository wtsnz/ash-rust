use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use ash_core::{Actor, Context, Error, Resource, SyncFnNotifier, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

resource! {
    resource CommunicationService;
    table "communication_services";

    attributes {
        id: Uuid [pk],
        name: String,
    }

    actions {
        create create {
            primary;
            accept [name];
        }

        read read {
            primary;
        }

        // 1. Ash Elixir style `action <name>, <return_type>` with inline run closure:
        action send_message, bool {
            argument recipient: String;
            argument message: String;
            argument priority: Option<String>;

            run |input| async move {
                // Accessing input fields and context/actor:
                let is_urgent = input.priority.as_deref() == Some("high");
                let _sender = input.actor();
                let _has_recipient = !input.recipient.is_empty();
                let _has_body = !input.message.is_empty();

                Ok(is_urgent)
            }
        }

        // 2. Generic action returning a computation (i64):
        generic calculate_total, i64 {
            argument quantity: i64;
            argument unit_price: i64;
            argument discount: Option<i64>;

            run |input| async move {
                let base = input.quantity * input.unit_price;
                let final_amount = base - input.discount.unwrap_or(0);
                Ok(final_amount)
            }
        }

        // 3. Generic action without inline run (runner provided dynamically via `.run(...)`):
        action dynamic_operation, String {
            argument payload: String;
        }

        // 4. Generic action protected by policy:
        action admin_purge, bool {
            argument reason: String;

            run |_input| async move {
                Ok(true)
            }
        }
    }

    policies {
        // Allow public access to read and create:
        policy action_type(read) | action(create) | action(send_message) | action(calculate_total) | action(dynamic_operation) {
            authorize_if always;
        }

        // Restrict admin_purge to admin role:
        policy action(admin_purge) {
            authorize_if actor_attribute_equals(role, "admin");
        }
    }
}

#[tokio::test]
async fn test_generic_action_send_message_execution() {
    let ctx = Context::new(Memory::new());

    // 1. Call with optional argument provided:
    let is_urgent = CommunicationService::send_message(&ctx)
        .recipient("user@example.com")
        .message("System update")
        .priority("high")
        .await
        .unwrap();

    assert!(is_urgent);

    // 2. Call with optional argument omitted (defaults to None):
    let not_urgent = CommunicationService::send_message(&ctx)
        .recipient("user@example.com")
        .message("Weekly newsletter")
        .await
        .unwrap();

    assert!(!not_urgent);
}

#[tokio::test]
async fn test_generic_action_missing_required_argument_fails() {
    let ctx = Context::new(Memory::new());

    // Omit required `message` argument:
    let err = CommunicationService::send_message(&ctx)
        .recipient("user@example.com")
        .await
        .unwrap_err();

    match err {
        Error::Missing { field } => assert_eq!(field, "message"),
        other => panic!("expected Error::Missing, got: {other:?}"),
    }
}

#[tokio::test]
async fn test_generic_action_numeric_computation() {
    let ctx = Context::new(Memory::new());

    let total = CommunicationService::calculate_total(&ctx)
        .quantity(5)
        .unit_price(20)
        .discount(15)
        .await
        .unwrap();

    assert_eq!(total, 85); // 5 * 20 - 15 = 85

    let total_no_discount = CommunicationService::calculate_total(&ctx)
        .quantity(4)
        .unit_price(25)
        .await
        .unwrap();

    assert_eq!(total_no_discount, 100);
}

#[tokio::test]
async fn test_generic_action_dynamic_runner() {
    let ctx = Context::new(Memory::new());

    // Action defined without an inline run block, supplied at call time via `.run(...)`:
    let result = CommunicationService::dynamic_operation(&ctx)
        .payload("test input")
        .run(|input| async move {
            let reversed = input.payload.chars().rev().collect::<String>();
            Ok(format!("PROCESSED: {reversed}"))
        })
        .await
        .unwrap();

    assert_eq!(result, "PROCESSED: tupni tset");
}

#[tokio::test]
async fn test_generic_action_policy_authorization() {
    let ctx = Context::new(Memory::new());

    // 1. Without admin actor: forbidden
    let err = CommunicationService::admin_purge(&ctx)
        .reason("Cleanup")
        .await
        .unwrap_err();

    assert!(matches!(err, Error::Forbidden));

    // 2. With admin actor: authorized
    let admin = Actor::new(Uuid::new_v4()).with_attr("role", "admin");
    let admin_ctx = ctx.with_actor(admin);

    let purged = CommunicationService::admin_purge(&admin_ctx)
        .reason("Scheduled maintenance")
        .await
        .unwrap();

    assert!(purged);
}

#[tokio::test]
async fn test_generic_action_notification_dispatch() {
    let notified = Arc::new(AtomicBool::new(false));
    let notified_clone = notified.clone();

    let notifier = SyncFnNotifier::new("test_generic_notifier", move |notif| {
        if notif.action == "send_message" {
            notified_clone.store(true, Ordering::SeqCst);
        }
        Ok(())
    });

    let ctx = Context::new(Memory::new()).with_notifier(Arc::new(notifier));

    CommunicationService::send_message(&ctx)
        .recipient("alert@example.com")
        .message("Fire drill")
        .await
        .unwrap();

    assert!(notified.load(Ordering::SeqCst));
}

#[tokio::test]
async fn test_generic_action_in_sqlite_context() {
    let sqlite = Sqlite::memory().await.unwrap();
    let ctx = Context::new(sqlite);
    ctx.install(&[&CommunicationService::DEF]).await.unwrap();

    let total = CommunicationService::calculate_total(&ctx)
        .quantity(10)
        .unit_price(12)
        .discount(20)
        .await
        .unwrap();

    assert_eq!(total, 100);
}
