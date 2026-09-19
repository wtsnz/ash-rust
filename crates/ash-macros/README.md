# ash-macros

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-macros/badge.svg)](https://docs.rs/ash-macros)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-macros` provides procedural macros that empower **ash-rust** with an expressive, declarative Domain Specific Language (DSL) inspired by Elixir's [Ash Framework](https://ash-hq.org).

It eliminates repetitive boilerplate by generating strongly-typed structs, static `ResourceDef` metadata, compile-time field operators, fluent action invocation builders, and domain code interfaces.

---

## What This Crate Provides

- **`resource!` (and `define!`) Macro**:
  - Declarative definition of attributes, identities, embedded resources, timestamps, relationships (`belongs_to`, `has_one`, `has_many`, `many_to_many`), calculations (including `(arg: Type)` and `arg(name)`), aggregates, and field-level policies.
  - DRY action input declarations (`accept [field1, field2]`) where types are inferred directly from resource attributes.
  - Query preparations on read actions (`prepare filter(...)`, `prepare sort(...)`, `prepare limit(...)`, `prepare offset(...)`).
  - Rich expressions in calculations (`total = price * quantity`, `display = coalesce(...)`, `badge = if_else(...)`).
  - Custom action arguments (`argument name: Type`), validations (`present`, `string_length`, `numericality`, `one_of`), and changes (`set`, `set_new`).
  - Optimistic locking configuration (`[version]`).
  - Per-resource storage (`store SqliteStore;` / `MemoryStore` / `PostgresStore`, or a custom `StoreTag`).
  - Extension token forwarding (`extend <macro>! { ... }`) for zero-coupling 3rd-party integrations (Pattern 1).
- **`domain!` Macro**:
  - Groups related resources into a bounded context.
  - Generates typed code interfaces (`helpdesk.open_ticket(...)`).
  - Provides domain-level transactions and cross-resource schema initialization (`domain.install()`).
- **`#[derive(Resource)]`**:
  - Derive macro for annotated struct-based resource declarations.
- **Generated Ergonomic APIs**:
  - **Zero-Import Field Operators**: `Article::title.eq("Rust")` directly on the resource struct.
  - **Typed Action Builders**: `Article::publish(&ctx).title("...").tag("rust").await?`.
  - **Zero-Import Action Invocations**: Update and destroy actions accept IDs, records, and record references (`Ticket::assign(&ctx, id)`, `Ticket::assign(&ctx, &ticket)`, `Ticket::destroy(&ctx, &ticket)`) without importing action traits.
  - **IDE Autocomplete & Diagnostics**: Slot-specific typecheck probes complete `accept [...]` (including empty `[]`), relationship names, and `policy action(...)` without offering every method on the resource. Jump-to-definition and rename still follow the original tokens. Kind-gate errors underline the illegal keyword. Wrong-slot names (`accept` a relationship, `filter` a relationship) are called out explicitly. Filter field typos get "Did you mean?". Calculation `string_length` on a non-string is a named error; arithmetic is type-checked. `relates_to` / `is_nil` / `eq` and `custom` / `before_action` / `func` paths type-check on the original tokens. `notifiers` and `extensions` entries are `&'static dyn Notifier` / `ResourceExtension`. Aggregate `filter:` fields are probed on the destination resource. Semantic lints (`present` on a required field, accept overwritten by `set`, unused arguments) show as rustc warnings. Incomplete `domain!` bodies still complete resource and action names.
  - **Intelligent Error Guidance**: Levenshtein distance suggestions ("Did you mean?") for section typos, action kinds, validations, and field references.
  - **Changeset Interoperability**: Generated action builders automatically implement `IntoChangeset` for direct use in `Multi` pipelines.

---

## Quick Example

### 1. Defining a Resource with `resource!`

```rust
use ash_core::resource;
use uuid::Uuid;

resource! {
    Product {
        table "products";

        attributes {
            id: Uuid [pk];
            sku: String;
            name: String;
            price: i64;
            description: Option<String>;
            active: bool;
        }

        actions {
            create create {
                primary;
                accept [sku, name, price, description];
                change set(active = true);
                validate present(sku);
                validate numericality(price, min: 1);
            }

            read read {
                primary;
            }

            update deactivate {
                change set(active = false);
            }
        }
    }
}
```

### 2. Defining a Domain with `domain!`

```rust
use ash_core::domain;

domain! {
    Catalog {
        resources {
            Product {
                define create_product action: create;
                define get_product action: read;
            };
        }
    }
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
