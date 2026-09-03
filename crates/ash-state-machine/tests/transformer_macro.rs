//! # Pattern 2: Transformative Macro Decorator (`#[state_machine] resource! { ... }`)
//!
//! ## Overview & Use Cases
//!
//! In Elixir's Ash Framework, `AshStateMachine` acts as a compile-time Spark DSL
//! *transformer*. It inspects the `state_machine { ... }` declaration, injects the
//! state attribute into the resource, injects lifecycle transitions and validations
//! into the actions, and registers extension metadata.
//!
//! Pattern 2 brings this exact experience to Rust via an outer attribute macro:
//!
//! ### Key Capabilities Demonstrated in this Test:
//! 1. **Auto-Injected Attribute**: Notice that `status: String` is **not** written inside
//!    `attributes { ... }`. The `#[state_machine]` transformer injects it automatically!
//! 2. **Auto-Injected Action Validations & Changes**:
//!    - On create, the initial state (`"pending"`) is set automatically.
//!    - On `submit`, `pay`, and `cancel`, the allowed `from` states and target `to` states
//!      are enforced and updated automatically without manual action wiring.
//! 3. **Auto-Generated Helper Methods**:
//!    - `order.current_state()`
//!    - `order.can_submit()`, `order.can_pay()`, `order.can_cancel()`
//!    - `order.possible_next_states("pending")`
//! 4. **Runtime Introspection**:
//!    - `Order::DEF.extension::<StateMachineDef>()` retrieves the state machine metadata.

use ash_core::{Context, Error, Resource};
use ash_memory::Memory;
use ash_state_machine::{InvalidTransition, StateMachineDef, state_machine};
use uuid::Uuid;

#[state_machine]
resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        // Note: `status` is omitted here! The transformer injects `status: String`.
        amount: i64,
    }

    state_machine {
        state_attribute status;
        initial: "pending";
        transition submit, from: ["pending"], to: "submitted";
        transition pay, from: ["submitted"], to: "paid";
        transition cancel, from: ["pending", "submitted"], to: "cancelled";
    }

    actions {
        create create {
            primary;
            accept {
                amount: i64,
            }
        }

        read read {
            primary;
        }

        // Action validations and target state changes are injected automatically!
        update submit {}
        update pay {}
        update cancel {}
    }
}

#[tokio::test]
async fn test_pattern2_transformer_macro_lifecycle_and_helpers() {
    let ctx = Context::new(Memory::new());

    // 1. Create: verify auto-injected initial state "pending"
    let order = Order::create(&ctx)
        .amount(250)
        .await
        .expect("create order");

    assert_eq!(order.current_state(), "pending");
    assert_eq!(order.status, "pending");
    assert_eq!(order.amount, 250);

    // 2. Verify auto-generated state check helpers
    assert!(order.can_submit(), "should be able to submit when pending");
    assert!(!order.can_pay(), "cannot pay directly from pending");
    assert!(order.can_cancel(), "can cancel from pending");

    let next_states = order.possible_next_states();
    assert_eq!(next_states, vec!["submitted", "cancelled"]);

    // 3. Attempt invalid transition: pay from pending -> must fail with InvalidTransition error!
    let invalid_pay_res = Order::pay(&ctx, order.id)
        .await;

    match invalid_pay_res {
        Err(Error::Extension(ext_err)) => {
            let transition_err = ext_err
                .downcast_ref::<InvalidTransition>()
                .expect("expected InvalidTransition error");
            assert_eq!(transition_err.current_state, "pending");
            assert_eq!(transition_err.target_state, "paid");
            assert_eq!(transition_err.action, "pay");
        }
        other => panic!("expected Error::Extension(InvalidTransition), got: {other:?}"),
    }

    // 4. Valid transition: submit -> moves to "submitted"
    let submitted_order = Order::submit(&ctx, order.id)
        .await
        .expect("submit succeeds");

    assert_eq!(submitted_order.current_state(), "submitted");
    assert!(!submitted_order.can_submit());
    assert!(submitted_order.can_pay());
    assert!(submitted_order.can_cancel());

    // 5. Valid transition: pay -> moves to "paid"
    let paid_order = Order::pay(&ctx, order.id)
        .await
        .expect("pay succeeds");

    assert_eq!(paid_order.current_state(), "paid");
    assert!(!paid_order.can_submit());
    assert!(!paid_order.can_pay());
    assert!(!paid_order.can_cancel());

    // 6. Runtime Introspection: verify StateMachineDef is registered in ResourceDef.extensions
    let sm_ext = Order::DEF
        .extension::<StateMachineDef>()
        .expect("StateMachineDef must be registered in ResourceDef extensions");

    assert_eq!(sm_ext.state_attribute, "status");
    assert_eq!(sm_ext.default_initial_state, Some("pending"));
    assert_eq!(sm_ext.transitions.len(), 3);
}

#[tokio::test]
async fn test_pattern2_transformer_macro_cancellation_flow() {
    let ctx = Context::new(Memory::new());

    let order = Order::create(&ctx)
        .amount(500)
        .await
        .expect("create order");

    // Cancel directly from pending
    let cancelled = Order::cancel(&ctx, order.id)
        .await
        .expect("cancel succeeds");

    assert_eq!(cancelled.current_state(), "cancelled");
    assert!(!cancelled.can_submit());
    assert!(!cancelled.can_pay());
    assert!(!cancelled.can_cancel());
}

// Verify that transitions not explicitly declared in `actions` are auto-synthesized!
mod ticket_module {
    use super::*;

    #[state_machine]
    resource! {
        resource Ticket;
        table "tickets";

        attributes {
            id: Uuid [pk],
            title: String,
            resolution_note: Option<String>,
        }

        state_machine {
            state_attribute status;
            initial: "open";
            // `resolve` has explicit action with accept { resolution_note: Option<String> }
            transition resolve, from: ["open"], to: "resolved";
            // `close` is NOT declared in actions below - it will be auto-synthesized by the transformer!
            transition close, from: ["open", "resolved"], to: "closed";
        }

        actions {
            create create {
                primary;
                accept {
                    title: String,
                }
            }

            read read {
                primary;
            }

            update resolve {
                accept {
                    resolution_note: Option<String>,
                }
            }
        }
    }

    #[tokio::test]
    async fn test_pattern2_auto_synthesized_actions_and_custom_accept() {
        let ctx = Context::new(Memory::new());

        let ticket = Ticket::create(&ctx)
            .title("Bug in checkout")
            .await
            .expect("create ticket");

        assert_eq!(ticket.current_state(), "open");
        assert_eq!(ticket.status, "open");

        // 1. Resolve with custom accept parameter
        let resolved = Ticket::resolve(&ctx, ticket.id)
            .resolution_note(Some("Fixed in commit 1234".into()))
            .await
            .expect("resolve ticket");

        assert_eq!(resolved.current_state(), "resolved");
        assert_eq!(resolved.resolution_note.as_deref(), Some("Fixed in commit 1234"));

        // 2. Close using the auto-synthesized `close` action
        let closed = Ticket::close(&ctx, ticket.id)
            .await
            .expect("auto-synthesized close action succeeds");

        assert_eq!(closed.current_state(), "closed");
    }
}
