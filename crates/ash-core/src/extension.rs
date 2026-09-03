//! # Extensibility in `ash-rust`
//!
//! In Elixir's Ash Framework, extensions are powered by the **Spark DSL**, which uses
//! compile-time *transformers* to inspect DSL schemas, inject attributes, and register
//! validations across modules.
//!
//! In Rust, proc-macro crates are statically compiled, so `ash-core` and `ash-macros` cannot
//! dynamically scan loaded crates. Instead, `ash-rust` provides two decoupled extension
//! patterns that deliver the same declarative power:
//!
//! ---
//!
//! ## Pattern 1: Additive Token-Forwarding (`extend <macro_path>! { ... }`)
//!
//! **Best for**: Additive or companion tools that do *not* need to rewrite attributes or actions.
//! Examples include:
//! - Audit trail generation (e.g. `audit!`)
//! - Search engine indexing (e.g. `searchable!`)
//! - GraphQL / OpenAPI schema generators
//! - Companion traits and custom metadata registration
//!
//! ### How it works:
//! Inside `resource!`, you can add one or more `extend` blocks:
//! ```text
//! resource! {
//!     resource Product;
//!     table "products";
//!
//!     attributes {
//!         id: Uuid [pk],
//!         name: String,
//!         price: i64,
//!     }
//!
//!     actions {
//!         create create { primary }
//!         read read { primary }
//!     }
//!
//!     // Forwarded verbatim to the external macro!
//!     extend my_audit::audit! {
//!         track: [price];
//!     }
//! }
//! ```
//!
//! `ash-macros` does not need to know anything about `my_audit`. During code generation,
//! `resource!` generates the standard resource code and appends:
//! ```text
//! my_audit::audit!(Product, {
//!     track: [price];
//! });
//! ```
//! The external macro receives the resource identifier (`Product`) and the raw tokens inside
//! the block, allowing it to generate companion trait implementations, metadata, or event listeners.
//!
//! ---
//!
//! ## Pattern 2: Transformative Macro Decorator (`#[transformer] resource! { ... }`)
//!
//! **Best for**: Extensions that need to **mutate the resource AST** before compilation.
//! Examples include:
//! - State Machines (`#[ash_state_machine::state_machine]`): auto-injects the state attribute
//!   (e.g. `status: String`), transition validations, and state changes into action definitions.
//! - Soft Delete / Archival (`#[ash_archival::archival]`): auto-injects `archived_at: Option<String>`
//!   and wraps default read queries with `is_nil(archived_at)`.
//! - Multi-tenancy / Organization scoping.
//!
//! ### How it works:
//! The outer attribute macro intercepts the `resource! { ... }` AST before `ash-macros` runs:
//! ```text
//! #[ash_state_machine::state_machine]
//! resource! {
//!     resource Order;
//!     table "orders";
//!
//!     attributes {
//!         id: Uuid [pk],
//!         amount: i64,
//!         // `status` is auto-injected by the state machine transformer!
//!     }
//!
//!     state_machine {
//!         state_attribute status;
//!         initial: "pending";
//!         transition pay, from: ["pending"], to: "paid";
//!     }
//!
//!     actions {
//!         create create { primary }
//!         update pay {} // `status = "paid"` change is auto-injected!
//!     }
//! }
//! ```
//!
//! The transformer macro strips its custom DSL block (`state_machine { ... }`), mutates the
//! AST (injecting attributes, action changes, or validations), and emits the transformed tokens
//! into `::ash_core::resource! { ... }`.
//!
//! Because Rust attribute macros evaluate outside-in, multiple transformers can be chained
//! in sequence—mirroring Spark DSL transformer pipelines with zero coupling to `ash-core`.
//!
//! ---
//!
//! ## Runtime Introspection: `ResourceExtension` Trait
//!
//! At runtime, extensions can store type-erased metadata on `ResourceDef.extensions`.
//! Any type implementing [`ResourceExtension`] can be queried via [`ResourceDef::extension`].

use std::any::Any;
use std::fmt::Debug;

/// Trait implemented by extension metadata structs stored in [`crate::ResourceDef::extensions`].
pub trait ResourceExtension: Send + Sync + Debug + 'static {
    /// Identifier or name for the extension (e.g. `"StateMachine"`, `"AuditTrail"`).
    fn name(&self) -> &'static str;

    /// Downcasting helper to retrieve the concrete extension struct.
    fn as_any(&self) -> &dyn Any;
}
