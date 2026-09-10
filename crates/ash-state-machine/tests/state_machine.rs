use ash_core::{Context, Error, Resource, resource};
use ash_memory::Memory;
use ash_state_machine::{
    DefaultStateChange, HasStateMachine, InvalidTransition, StateMachineDef, TransitionChange,
    TransitionDef, TransitionValidation,
};
use uuid::Uuid;

static ORDER_STATE_MACHINE: StateMachineDef = StateMachineDef::new(
    "status",
    &["draft", "pending"],
    Some("draft"),
    &[
        TransitionDef::new("submit", &["draft"], "pending"),
        TransitionDef::new("approve", &["pending"], "approved"),
        TransitionDef::new("reject", &["pending"], "rejected"),
        TransitionDef::new("cancel", &["draft", "pending"], "cancelled"),
    ],
);

static SUBMIT_VALIDATION: TransitionValidation =
    TransitionValidation::new("status", &["draft"], "pending", "submit");
static SUBMIT_CHANGE: TransitionChange = TransitionChange::new("status", "pending");

static APPROVE_VALIDATION: TransitionValidation =
    TransitionValidation::new("status", &["pending"], "approved", "approve");
static APPROVE_CHANGE: TransitionChange = TransitionChange::new("status", "approved");

static REJECT_VALIDATION: TransitionValidation =
    TransitionValidation::new("status", &["pending"], "rejected", "reject");
static REJECT_CHANGE: TransitionChange = TransitionChange::new("status", "rejected");

static CANCEL_VALIDATION: TransitionValidation =
    TransitionValidation::new("status", &["draft", "pending"], "cancelled", "cancel");
static CANCEL_CHANGE: TransitionChange = TransitionChange::new("status", "cancelled");

static DEFAULT_DRAFT_CHANGE: DefaultStateChange = DefaultStateChange::new("status", "draft");

resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        total: i64,
        status: String,
    }

    extensions [
        &ORDER_STATE_MACHINE,
    ]

    actions {
        create draft {
            primary;
            accept [total];
            change custom(&DEFAULT_DRAFT_CHANGE);
        }

        update submit {
            validate custom(&SUBMIT_VALIDATION);
            change custom(&SUBMIT_CHANGE);
        }

        update approve {
            validate custom(&APPROVE_VALIDATION);
            change custom(&APPROVE_CHANGE);
        }

        update reject {
            validate custom(&REJECT_VALIDATION);
            change custom(&REJECT_CHANGE);
        }

        update cancel {
            validate custom(&CANCEL_VALIDATION);
            change custom(&CANCEL_CHANGE);
        }
    }
}

impl HasStateMachine for Order {
    const STATE_MACHINE: StateMachineDef = ORDER_STATE_MACHINE;

    fn current_state(&self) -> &str {
        &self.status
    }
}

#[tokio::test]
async fn test_state_machine_extension_introspection() {
    // Introspect extension via Order::DEF without ash-core knowing about StateMachineDef
    let ext = Order::DEF
        .extension::<StateMachineDef>()
        .expect("extension should be attached");
    assert_eq!(ext.state_attribute, "status");
    assert_eq!(ext.default_initial_state, Some("draft"));
}

#[tokio::test]
async fn test_state_machine_default_initial_and_valid_lifecycle() {
    let ctx = Context::new(Memory::new());

    // 1. Create order: status defaults to "draft"
    let order = Order::draft(&ctx)
        .total(150)
        .await
        .expect("order should be created");

    assert_eq!(order.current_state(), "draft");
    assert!(order.can_transition("submit"));
    assert!(order.can_transition("cancel"));
    assert!(!order.can_transition("approve"));
    assert!(!order.can_transition("reject"));

    let next_states = order.possible_next_states();
    assert!(next_states.contains(&"pending"));
    assert!(next_states.contains(&"cancelled"));
    assert!(!next_states.contains(&"approved"));

    // 2. Transition draft -> pending via submit action
    let submitted = order.submit_on(&ctx).await.expect("submit should succeed");
    assert_eq!(submitted.current_state(), "pending");
    assert!(submitted.can_transition("approve"));
    assert!(submitted.can_transition("reject"));
    assert!(submitted.can_transition("cancel"));
    assert!(!submitted.can_transition("submit"));

    // 3. Transition pending -> approved via approve action
    let approved = submitted.approve_on(&ctx).await.expect("approve should succeed");
    assert_eq!(approved.current_state(), "approved");
    assert!(!approved.can_transition("submit"));
    assert!(!approved.can_transition("approve"));
    assert!(!approved.can_transition("cancel"));
    assert!(approved.possible_next_states().is_empty());
}

#[tokio::test]
async fn test_state_machine_invalid_transition_rejected() {
    let ctx = Context::new(Memory::new());

    let order = Order::draft(&ctx)
        .total(200)
        .await
        .expect("order should be created");

    // Attempting to approve directly from draft should fail with Error::Extension containing InvalidTransition
    let err = order
        .approve_on(&ctx)
        .await
        .expect_err("draft order cannot be approved directly");

    match err {
        Error::Extension(ext_err) => {
            let inv = ext_err
                .downcast_ref::<InvalidTransition>()
                .expect("expected InvalidTransition");
            assert_eq!(inv.current_state, "draft");
            assert_eq!(inv.target_state, "approved");
            assert_eq!(inv.action, "approve");
        }
        other => panic!("expected Error::Extension, got {other:?}"),
    }
}

#[tokio::test]
async fn test_state_machine_multi_source_transition() {
    let ctx = Context::new(Memory::new());

    // Cancel from draft
    let order1 = Order::draft(&ctx).total(50).await.unwrap();
    let cancelled1 = order1.cancel_on(&ctx).await.expect("cancel from draft should work");
    assert_eq!(cancelled1.current_state(), "cancelled");

    // Cancel from pending
    let order2 = Order::draft(&ctx).total(75).await.unwrap();
    let pending2 = order2.submit_on(&ctx).await.unwrap();
    let cancelled2 = pending2.cancel_on(&ctx).await.expect("cancel from pending should work");
    assert_eq!(cancelled2.current_state(), "cancelled");

    // Attempting to cancel already cancelled order should fail
    let err = cancelled2.cancel_on(&ctx).await.expect_err("cannot cancel when already cancelled");
    match err {
        Error::Extension(ext_err) => {
            let inv = ext_err
                .downcast_ref::<InvalidTransition>()
                .expect("expected InvalidTransition");
            assert_eq!(inv.current_state, "cancelled");
            assert_eq!(inv.target_state, "cancelled");
            assert_eq!(inv.action, "cancel");
        }
        other => panic!("expected Error::Extension, got {other:?}"),
    }
}
