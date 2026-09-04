# Ash-Rust Documentation

`ash-rust` is a declarative, resource-oriented framework for Rust inspired by [Elixir's Ash Framework](https://ash-hq.org/). It models your application as declarative **Resources** grouped into bounded **Domains**, decoupling business rules, authorization, and workflows from underlying storage layers.

---

## Documentation Index

1. **[Architecture & Philosophy](architecture.md)**
   - Core design principles
   - Crate structure & decoupling
   - Data layers (`ash-memory`, `ash-sqlite`, and beyond)
   - Spark DSL vs. Rust macro engine

2. **[Extensibility Guide](extensions.md)**
   - The two extension patterns:
     - **Pattern 1: Additive Token-Forwarding (`extend <macro>! { ... }`)**
     - **Pattern 2: Transformative Macro Decorator (`#[transformer] resource! { ... }`)**
   - The 3 decoupling extension points (`CustomValidation`, `CustomChange`, `ResourceExtension`)
   - Decoupled error handling with `Error::Extension`

3. **[DSL & Modeling Guide](dsl-guide.md)**
   - `resource!` macro reference
   - Attributes & types
   - Relationships (`belongs_to`, `has_many`, `many_to_many`)
   - Calculations & Aggregates (`count`, `exists`, `sum`, `first`)
   - Actions (`create`, `read`, `update`, `destroy`, `generic`)
   - Action Arguments (non-attribute inputs)
   - `domain!` macro reference & Code Interfaces

4. **[Advanced Capabilities](features.md)**
   - **Ash.Multi & Atomic Transactions**: Multi-step pipelines with rollback
   - **State Machines**: Declarative lifecycles with `#[state_machine]`
   - **Optimistic Locking**: Concurrency control via `[version]`
   - **Pagination**: Offset and keyset cursor-based pagination with `Page<T>`
   - **Authorization & Policies**: Actor checks, field-level policies, and data redaction

5. **[Performance Benchmarks](benchmarks.md)**
   - Empirical results vs. canonical Ash Framework in Elixir
   - Action, query, and aggregate performance breakdown
   - Architectural root cause analysis (monomorphism, stack vs. heap allocation)
   - Continuous regression testing with Criterion (`cargo bench`)

6. **[Relational Query Patterns & Edge Cases](query-patterns-and-edge-cases.md)**
   - Common patterns: Keyset & offset pagination, combinatorial filters, graph batch loading
   - Relational edge cases: Empty `IN ()` clauses, SQL 3-valued null logic, self-referential shadowing
   - Engine matrix: SQLite (`ash-sqlite`) vs PostgreSQL (`ash-postgres`)
   - Test suite verification matrix

7. **[Authentication & Token Security (`ash-authentication`)](wip/0004-ash-authentication.md)**
   - RFC 0004 specification for declarative authentication
   - Strategies: Argon2id password hashing, JWT bearer tokens, and API key management
   - Axum integration: `AuthUser` extractor and `/auth` router
   - `#[authentication]` resource decorator macro
8. **Transactional Email & Action Notifiers (`ash-mailer`)**
   - Pluggable transactional email delivery (`MemoryMailer`, `ConsoleMailer`)
   - `EmailNotifier`: Declarative email delivery on committed resource actions
   - `AuthMailerSender`: Abstract bridge to `ash-authentication` for password resets, confirmations, and magic links


---

## Quick Example

```rust
use ash_core::{Context, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    resource Post;
    table "posts";

    attributes {
        id: Uuid [pk],
        title: String,
        views: i64,
        version: i64 [version], // Optimistic concurrency control
    }

    actions {
        create create {
            primary;
            accept {
                title: String,
            }
            change set_attribute(views, 0);
        }

        read read {
            primary;
        }

        update increment_views {
            argument by: i64;
            // Increments view count safely
            change custom(&MyIncrementChange);
        }
    }
}

#[tokio::test]
async fn run_post() {
    let ctx = Context::new(Memory::new());

    let post = Post::create(&ctx)
        .title("Getting Started with Ash-Rust")
        .await
        .unwrap();

    assert_eq!(post.title, "Getting Started with Ash-Rust");
    assert_eq!(post.views, 0);
    assert_eq!(post.version, 1);
}
```
