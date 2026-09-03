# Extensibility Guide

In Elixir's Ash Framework, extensions are powered by the **Spark DSL**, which uses compile-time *transformers* to inspect DSL schemas, inject attributes, and register validations across modules.

In Rust, proc-macro crates are statically compiled, meaning `ash-core` and `ash-macros` cannot dynamically scan arbitrary crates at runtime. Instead, `ash-rust` provides **3 core extension points** and **2 declarative macro patterns** to achieve the exact same declarative power without tight coupling.

---

## The 3 Decoupling Extension Points in `ash-core`

`ash-core` provides pluggable interfaces so external crates can execute custom logic during action pipelines:

### 1. `CustomValidation`
Allows external crates to validate changeset data before persistence:

```rust
pub trait CustomValidation: Send + Sync + 'static {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()>;
}

// In your resource action:
validation custom(&MyCustomValidator);
// or with a direct function:
validation func(|ctx| {
    if ctx.fields.get("name").is_none() {
        return Err(Error::Validation { field: "name".into(), message: "missing".into() });
    }
    Ok(())
});
```

The `ValidationContext` exposes:
- `ctx.record`: Existing record fields (for update/destroy actions).
- `ctx.fields`: Proposed mutated fields.
- `ctx.arguments`: Action arguments passed by the caller.

### 2. `CustomChange` & `change func(...)`
Allows external crates or local modules to mutate fields and attach dynamic lifecycle hooks during action execution:

```rust
pub trait CustomChange: Send + Sync + 'static {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()>;
}

// In your resource action:
// 1. Using a CustomChange struct (promotes to 'static directly with &MyStateChange):
change custom(&MyStateChange);

// 2. Or using a lightweight function (no struct or trait implementation needed):
change func(my_change_fn);
change func(|ctx| {
    ctx.fields.insert("updated_at".into(), Value::String(utc_now_iso8601()));
    Ok(())
});
```

The `ChangeContext` exposes:
- `ctx.fields`: Mutable reference to the proposed attribute field map.
- `ctx.actor`: The executing actor (`Option<&Actor>`).
- `ctx.arguments`: Read-only arguments passed to the action.
- `ctx.before_action(|fields| ...)`: Dynamically attach a pre-persistence hook.
- `ctx.after_action(|fields| ...)`: Dynamically attach a post-persistence hook.
- `ctx.after_transaction(|res| ...)`: Dynamically attach a post-transaction hook.

#### Example: Dynamic Lifecycle Hooks in a Reusable Plugin
```rust
struct AuditTracker;

impl CustomChange for AuditTracker {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        // Run before DB write:
        ctx.before_action(|fields| {
            fields.insert("version".into(), Value::Int(1));
            Ok(())
        });

        // Run after DB write:
        ctx.after_action(|fields| {
            println!("Record saved: {:?}", fields.get("id"));
            Ok(())
        });

        // Run when transaction commits or aborts:
        ctx.after_transaction(|res| {
            match res {
                Ok(fields) => println!("Committed {:?}", fields.get("id")),
                Err(err) => eprintln!("Transaction aborted: {err:?}"),
            }
        });

        Ok(())
    }
}
```

### 3. `ResourceExtension` & Open Error Handling
Allows third-party crates to store typed metadata on `ResourceDef` and return typed domain errors:

```rust
pub trait ResourceExtension: Send + Sync + Debug + 'static {
    fn name(&self) -> &'static str;
    fn as_any(&self) -> &dyn Any;
}

// Stored in ResourceDef:
resource.extension::<StateMachineDef>() // Option<&'static StateMachineDef>

// Custom errors wrapped cleanly in ash-core:
Error::Extension(Box<dyn std::error::Error + Send + Sync>)
```

---

## Pattern 1: Additive Token-Forwarding (`extend`)

**Best for**: Additive features, companion traits, schemas, and event listeners that do **not** need to rewrite resource attributes or actions.

### How it works
Inside `resource!`, you can add one or more `extend` blocks referencing any macro path:

```rust
use ash_core::resource;

resource! {
    resource Product;
    table "products";

    attributes {
        id: Uuid [pk],
        title: String,
        price: i64,
    }

    actions {
        create create { primary; accept { title: String, price: i64 } }
        read read { primary; }
    }

    // Pattern 1: Forwarded to 3rd-party macro
    extend audit_trail! {
        track: [price];
    }

    extend searchable! {
        index: "products_v1";
    }
}
```

### Macro Expansion
`ash-macros` compiles the resource normally and appends verbatim macro invocations:

```rust
audit_trail!(Product, {
    track: [price];
});

searchable!(Product, {
    index: "products_v1";
});
```

The external macro receives the resource identifier (`Product`) and the raw tokens inside the block, allowing it to generate companion traits (e.g. `Product::tracked_fields()`) without `ash-core` needing any knowledge of the extension.

---

## Pattern 2: Transformative Macro Decorator (`#[transformer]`)

**Best for**: Extensions that need to **mutate the resource AST** before compilation.
Examples include:
- **State Machines**: Auto-injects `status: String`, transitions, validations, and state changes.
- **Soft Delete / Archival**: Auto-injects `archived_at: Option<String>` and rewrites read filters.
- **Automatic Timestamps**: Auto-injects `inserted_at` and `updated_at`.

### How it works
The outer attribute macro wraps `resource! { ... }` and intercepts its tokens before `ash-macros` processes them:

```rust
use ash_state_machine::state_machine;

#[state_machine]
resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        amount: i64,
        // `status` is omitted; the transformer automatically injects it!
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
            accept { amount: i64 }
        }

        read read { primary; }

        // Validations and state changes are injected automatically!
        update submit {}
        update pay {}
        // Omitted actions (like cancel) are automatically synthesized!
    }
}
```

### What `#[state_machine]` Automates:
1. **Attribute Injection**: Injects `status: String` into `attributes { ... }`.
2. **Initial State Injection**: Injects default state change into the primary `create` action.
3. **Transition Wiring**: Injects `TransitionValidation` and `TransitionChange` into each transition action.
4. **Action Synthesis**: If an action is not declared in `actions`, it synthesizes `update <action> { ... }`.
5. **Metadata Registration**: Adds `StateMachineDef` into `extensions { ... }`.
6. **Helper Methods**: Implements `HasStateMachine` and generates `order.current_state()`, `order.can_submit()`, `order.possible_next_states()`.

---

## Comparison Table

| Dimension | Pattern 1: Token Forwarding (`extend`) | Pattern 2: Transformer Decorator (`#[transformer]`) |
| :--- | :--- | :--- |
| **Invocation** | `extend <path>! { ... }` inside `resource!` | `#[path] resource! { ... }` |
| **AST Mutation** | **No** (purely additive) | **Yes** (modifies attributes, actions, extensions) |
| **Coupling** | Zero (token passthrough) | Zero (emits standard `::ash_core::resource!`) |
| **Best Used For** | Audit logs, search indexing, companion traits, schemas | State machines, soft delete, multi-tenancy scoping, timestamps |
| **Chaining** | Multiple `extend` blocks | Multiple attribute decorators evaluated outside-in |
