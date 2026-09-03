# ash-state-machine

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-state-machine/badge.svg)](https://docs.rs/ash-state-machine)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-state-machine` provides declarative, validated state machines for **ash-rust** resources, modeled after Elixir's [`ash_state_machine`](https://github.com/ash-project/ash_state_machine).

It demonstrates **Pattern 2: The Macro Decorator / Transformer** architecture, where an external extension seamlessly injects state attributes, action validations, transition changes, and introspection metadata into standard `resource!` declarations without requiring any coupling in `ash-core`.

---

## What This Crate Provides

- **Declarative State Machine Modeling**:
  - Configure target state attributes, initial default states, and legal transitions.
  - Supports single or multiple source states per transition (`from: ["draft", "submitted"]`).
- **Compile-Time AST Transformation (`#[state_machine]`)**:
  - Automatically injects the state attribute into the resource's `attributes` block.
  - Automatically injects default initial state initialization into `create` actions.
  - Injects pre-commit `TransitionValidation` and `TransitionChange` hooks into transition actions.
- **Strict Transition Enforcement**:
  - Enforces valid lifecycle state changes before database persistence.
  - Returns `Error::Extension(InvalidTransition)` when illegal state transitions are attempted.
- **Runtime Introspection & Helpers (`HasStateMachine`)**:
  - Stores `StateMachineDef` in `ResourceDef.extensions` for schema and tooling introspection.
  - Generates type-safe instance methods:
    - `order.can_transition_to("cancelled")`
    - `order.can_pay()`
    - `order.possible_next_states()`

---

## Quick Example

```rust
use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_state_machine::state_machine;
use uuid::Uuid;

#[state_machine]
resource! {
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        subject: String,
        // `status: String` is automatically injected by #[state_machine]
    }

    state_machine {
        state_attribute status;
        initial: "open";
        transition assign, from: ["open"], to: "in_progress";
        transition resolve, from: ["in_progress"], to: "resolved";
        transition close, from: ["open", "in_progress", "resolved"], to: "closed";
    }

    actions {
        create open {
            primary;
            accept [subject];
            // Initial state "open" is automatically injected!
        }

        read read {
            primary;
        }

        // Action validations and state changes are automatically injected!
        update assign {}
        update resolve {}
        update close {}
    }
}

#[tokio::main]
async fn main() -> ash_core::Result<()> {
    let ctx = Context::new(Memory::new());

    // 1. Create a record (starts in "open" state)
    let ticket = Ticket::open(&ctx)
        .subject("Bug in production")
        .await?;
    assert_eq!(ticket.status, "open");

    // 2. Introspection helpers
    assert!(ticket.can_assign());
    assert!(!ticket.can_resolve()); // cannot jump directly from open to resolved!

    // 3. Valid transition
    let in_progress = ticket.assign_on(&ctx).await?;
    assert_eq!(in_progress.status, "in_progress");

    Ok(())
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
