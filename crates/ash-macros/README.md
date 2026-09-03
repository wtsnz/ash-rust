# ash-macros

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.0-orange.svg)](https://crates.io)
[![Documentation](https://docs.rs/ash-macros/badge.svg)](https://docs.rs/ash-macros)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

`ash-macros` provides procedural macros that empower **ash-rust** with an expressive, declarative Domain Specific Language (DSL) inspired by Elixir's [Ash Framework](https://ash-hq.org).

It eliminates repetitive boilerplate by generating strongly-typed structs, static `ResourceDef` metadata, compile-time field operators, fluent action invocation builders, and domain code interfaces.

---

## What This Crate Provides

- **`resource!` (and `define!`) Macro**:
  - Declarative definition of attributes, identities, embedded resources, timestamps, relationships (`belongs_to`, `has_many`, `many_to_many`), calculations, aggregates, and field-level policies.
  - DRY action input declarations (`accept [field1, field2]`) where types are inferred directly from resource attributes.
  - Query preparations on read actions (`prepare filter(...)`, `prepare sort(...)`, `prepare limit(...)`).
  - Rich expressions in calculations (`total = price * quantity`, `display = coalesce(...)`, `badge = if_else(...)`).
  - Custom action arguments (`argument name: Type`), validations (`present`, `string_length`, `numericality`, `one_of`), and changes (`set`, `set_new`).
  - Optimistic locking configuration (`[version]`).
  - Per-resource data layer declarations (`data_layer sqlite;` or `data_layer memory;`).
  - Extension token forwarding (`extend <macro>! { ... }`) for zero-coupling 3rd-party integrations (Pattern 1).
- **`domain!` Macro**:
  - Groups related resources into a bounded context.
  - Generates typed code interfaces (`order_domain.create_ticket(...)`).
  - Provides domain-level transactions and cross-resource schema initialization (`domain.install_schema()`).
- **`#[derive(Resource)]`**:
  - Derive macro for annotated struct-based resource declarations.
- **Generated Ergonomic APIs**:
  - **Zero-Import Field Operators**: `Article::title.eq("Rust")` directly on the resource struct.
  - **Typed Action Builders**: `Article::publish(&ctx).title("...").tag("rust").await?`.
  - **Changeset Interoperability**: Generated action builders automatically implement `IntoChangeset` for direct use in `Multi` pipelines.

---

## Quick Example

### 1. Defining a Resource with `resource!`

```rust
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource Product;
    table "products";

    attributes {
        id: Uuid [pk],
        sku: String,
        name: String,
        price: i64,
        description: Option<String>,
        active: bool,
    }

    actions {
        create create {
            primary;
            accept [sku, name, price, description];
            change set(active = true);
            validation present(sku);
            validation numericality(price, min: 1);
        }

        read read {
            primary;
        }

        update deactivate {
            change set(active = false);
        }
    }
}
```

### 2. Defining a Domain with `domain!`

```rust
use ash_core::domain;

domain! {
    domain Catalog {
        resources {
            Product {
                define: create_product, action: create;
                define: get_product, action: read;
            }
        }
    }
}
```

---

## License

Licensed under the [MIT License](../../LICENSE).
