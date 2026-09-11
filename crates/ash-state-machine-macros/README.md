# ash-state-machine-macros

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-state-machine-macros/badge.svg)](https://docs.rs/ash-state-machine-macros)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-state-machine-macros` provides procedural macro decorators for declarative state machine transformation in **ash-rust**.

It provides the `#[state_machine]` macro attribute that transforms standard `resource! { ... }` definitions by parsing an embedded `state_machine { ... }` block and synthesizing attributes, validations, changes, and metadata hooks at compile time.

---

## What This Crate Provides

- **`#[state_machine]` Macro Attribute**:
  - Implements **Pattern 2: The Macro Decorator / Transformer** architecture for `ash-rust`.
  - Parses an embedded `state_machine { ... }` block within a `resource! { ... }` declaration.
  - Automatically synthesizes:
    1. **Attribute Injection**: Adds the configured state attribute (e.g. `status: String`) into the resource's `attributes` block.
    2. **Creation Initialization**: Injects initial default state assignment into primary `create` actions.
    3. **Transition Hooks**: Injects `TransitionValidation` and `TransitionChange` hooks into `update` actions corresponding to defined state transitions.
    4. **Extension Registration**: Adds `ash_state_machine::def::StateMachineDef` into the resource's `extensions { ... }` block for runtime introspection.
    5. **Helper Code Generation**: Implements `HasStateMachine` for the generated resource, providing compile-time `can_<action>()` query helpers.

---

## Usage

This crate is typically consumed via the re-export in [`ash-state-machine`](../ash-state-machine):

```rust
use ash_core::resource;
use ash_state_machine::state_machine;
use uuid::Uuid;

#[state_machine]
resource! {
    Invoice {
        table "invoices";

        attributes {
            id: Uuid [pk];
            amount: i64;
        }

        state_machine {
            state_attribute status;
            initial: "draft";
            transition send_invoice, from: ["draft"], to: "sent";
            transition pay, from: ["sent"], to: "paid";
        }

        actions {
            create draft {
                primary;
                accept [amount];
            }

            read read {
                primary;
            }

            update send_invoice {}
            update pay {}
        }
    }
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
