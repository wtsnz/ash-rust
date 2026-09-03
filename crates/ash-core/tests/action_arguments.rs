use ash_core::{Context, Error, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource RefundRequest;
    table "refund_requests";

    attributes {
        id: Uuid [pk],
        order_id: String,
        amount: i64,
        status: String,
        reason: Option<String>,
    }

    actions {
        create submit {
            primary;
            accept {
                order_id: String,
                amount: i64,
            }
            argument reason: String;
            argument feedback: Option<String>;
            validate present(reason);
            validate string_length(reason, min = 5);
            change set(status = "submitted");
            change set_from_arg(reason, reason);
        }

        update approve {
            argument note: String;
            validate present(note);
            change set(status = "approved");
        }
    }
}

#[tokio::test]
async fn test_action_arguments_success_and_set_from_arg() {
    let ctx = Context::new(Memory::new());

    // Submit a refund request with required argument `reason`
    let refund = RefundRequest::submit(&ctx)
        .order_id("ORD-1234")
        .amount(99)
        .reason("Item arrived damaged")
        .await
        .expect("should submit successfully");

    assert_eq!(refund.order_id, "ORD-1234");
    assert_eq!(refund.amount, 99);
    assert_eq!(refund.status, "submitted");
    assert_eq!(refund.reason, Some("Item arrived damaged".into()));
}

#[tokio::test]
async fn test_action_arguments_missing_required_arg() {
    let ctx = Context::new(Memory::new());

    // Omitting the required argument `reason` should fail
    let err = RefundRequest::submit(&ctx)
        .order_id("ORD-1234")
        .amount(99)
        .await
        .expect_err("should fail due to missing required argument");

    match err {
        Error::Missing { field } => assert_eq!(field, "reason"),
        other => panic!("expected Error::Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn test_action_arguments_validation_failure() {
    let ctx = Context::new(Memory::new());

    // Providing reason with fewer than 5 characters should fail validation
    let err = RefundRequest::submit(&ctx)
        .order_id("ORD-1234")
        .amount(99)
        .reason("bad")
        .await
        .expect_err("should fail string_length validation on argument");

    match err {
        Error::Validation { field, message } => {
            assert_eq!(field, "reason");
            assert!(message.contains("at least 5 characters"));
        }
        other => panic!("expected Error::Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn test_action_arguments_on_update() {
    let ctx = Context::new(Memory::new());

    let refund = RefundRequest::submit(&ctx)
        .order_id("ORD-999")
        .amount(50)
        .reason("Defective part inside")
        .await
        .unwrap();

    // Update with required `note` argument
    let approved = refund
        .approve_on(&ctx)
        .note("Verified receipt and warehouse return")
        .await
        .expect("should approve");

    assert_eq!(approved.status, "approved");

    // Missing required argument on update
    let err = approved
        .approve_on(&ctx)
        .await
        .expect_err("should fail when note is missing");

    match err {
        Error::Missing { field } => assert_eq!(field, "note"),
        other => panic!("expected Error::Missing, got {other:?}"),
    }
}
