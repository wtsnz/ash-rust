//! # Ash State Machine
//!
//! Declarative state machines for `ash-rust` resources.
//!
//! ## Pattern 2: Transformative Macro Decorator (`#[state_machine] resource! { ... }`)
//!
//! In Elixir Ash, `AshStateMachine` acts as a compile-time Spark DSL transformer that
//! automatically injects state attributes, action validations, and action changes into
//! the resource definition.
//!
//! In `ash-rust`, this is achieved via the `#[state_machine]` attribute decorator:
//!
//! ```text
//! use ash_state_machine::state_machine;
//!
//! #[state_machine]
//! resource! {
//!     resource Order;
//!     table "orders";
//!
//!     attributes {
//!         id: Uuid [pk],
//!         amount: i64,
//!         // `status` is automatically injected by the transformer!
//!     }
//!
//!     state_machine {
//!         state_attribute status;
//!         initial: "pending";
//!         transition submit, from: ["pending"], to: "submitted";
//!         transition pay, from: ["submitted"], to: "paid";
//!         transition cancel, from: ["pending", "submitted"], to: "cancelled";
//!     }
//!
//!     actions {
//!         create create {
//!             primary;
//!             accept [amount];
//!         }
//!
//!         // Action validations and state changes are automatically injected!
//!         update submit {}
//!         update pay {}
//!         update cancel {}
//!     }
//! }
//! ```
//!
//! The transformer macro:
//! 1. Injects `status: String` into the `attributes { ... }` block.
//! 2. Injects the initial default state into the `create` action.
//! 3. Injects transition validations and target state changes into each transition action.
//! 4. Registers `StateMachineDef` in `ResourceDef.extensions` for runtime introspection.
//! 5. Implements [`HasStateMachine`] for `Order` and generates convenience query methods
//!    (`order.can_submit()`, `order.can_pay()`, `order.possible_next_states()`).

mod change;
pub mod def;
mod error;
mod trait_ext;
mod validation;

pub use ash_state_machine_macros::state_machine;
pub use change::{DefaultStateChange, TransitionChange};
pub use def::{StateMachineDef, TransitionDef};
pub use error::InvalidTransition;
pub use trait_ext::HasStateMachine;
pub use validation::TransitionValidation;

pub const fn transition_validation(
    state_attribute: &'static str,
    allowed_from: &'static [&'static str],
    target_state: &'static str,
    action: &'static str,
) -> TransitionValidation {
    TransitionValidation::new(state_attribute, allowed_from, target_state, action)
}

pub const fn transition_change(
    state_attribute: &'static str,
    target_state: &'static str,
) -> TransitionChange {
    TransitionChange::new(state_attribute, target_state)
}

pub const fn default_state_change(
    state_attribute: &'static str,
    default_state: &'static str,
) -> DefaultStateChange {
    DefaultStateChange::new(state_attribute, default_state)
}
