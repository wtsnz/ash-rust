# Advanced Capabilities

`ash-rust` includes enterprise-grade primitives for complex business workflows, concurrency control, pagination, authorization, and transactional pipelines.

---

## 1. Ash.Multi & Atomic Transactions

Inspired by Elixir's `Ash.Multi` and `Ecto.Multi`, `ash_core::Multi` lets you compose multi-step write pipelines that execute inside an atomic database transaction.

### Features
- Named steps (`create`, `update`, `destroy`, `run`, `run_async`).
- Dynamic steps receive a `MultiResult` containing results of prior steps.
- If any step fails or returns an `Err`, the entire transaction is rolled back.
- Supports nested savepoints in SQLite.

```rust
use ash_core::Multi;

let result = Multi::new()
    .create("create_workspace", Workspace::create(&ctx).name("Acme Corp"))
    .run("add_admin", |ctx, results| {
        let ws: &Workspace = results.get("create_workspace")?;
        Member::create(ctx)
            .workspace_id(ws.id)
            .role("admin")
            .commit()
    })
    .run_async("audit_log", |ctx, results| Box::pin(async move {
        // Asynchronous side-effect or notification step
        Ok(results.get::<Workspace>("create_workspace")?.id)
    }))
    .run_transaction(&ctx)
    .await?;

let workspace: &Workspace = result.get("create_workspace")?;
let member: &Member = result.get("add_admin")?;
```

---

## 2. Declarative State Machines

Implemented via `ash-state-machine` using Pattern 2 (Transformative Macro Decorator):

```rust
use ash_state_machine::state_machine;

#[state_machine]
resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        amount: i64,
        // `status: String` is auto-injected by #[state_machine]!
    }

    state_machine {
        state_attribute status;
        initial: "pending";
        transition submit, from: ["pending"], to: "submitted";
        transition pay, from: ["submitted"], to: "paid";
        transition cancel, from: ["pending", "submitted"], to: "cancelled";
    }

    actions {
        create create { primary; accept { amount: i64 } }
        read read { primary; }
        update submit {}
        update pay {}
        update cancel {}
    }
}
```

### Runtime Checks & Error Handling
```rust
let order = Order::create(&ctx).amount(100).await?;
assert_eq!(order.current_state(), "pending");

// Valid transitions succeed:
let submitted = order.submit().await?;
assert_eq!(submitted.current_state(), "submitted");

// Invalid transitions return typed Error::Extension:
let err = submitted.submit().await.unwrap_err();
match err {
    Error::Extension(e) => {
        let inv = e.downcast_ref::<InvalidTransition>().unwrap();
        println!("Cannot transition from {} to {} via {}", inv.current_state, inv.target_state, inv.action);
    }
    _ => unreachable!(),
}
```

---

## 3. Optimistic Locking

Prevents concurrent updates from overwriting each other by tagging an integer attribute with `[version]`:

```rust
resource! {
    resource BankAccount;
    table "bank_accounts";

    attributes {
        id: Uuid [pk],
        balance: i64,
        version: i64 [version],
    }

    actions {
        create open { primary; accept { balance: i64 } }
        update deposit { accept { balance: i64 } }
    }
}
```

### How it behaves:
1. When created, `version` defaults to `1`.
2. Every update increments the version attribute (`version + 1`).
3. In SQL / DataLayers, the update applies an atomic check:
   ```sql
   UPDATE bank_accounts SET balance = ?, version = ? WHERE id = ? AND version = ?
   ```
4. If another process modified the row in the meantime, the update affects `0` rows and returns:
   ```rust
   Error::StaleRecord { resource: "BankAccount", id: account.id }
   ```

---

## 4. Keyset & Offset Pagination

Ash queries support two high-performance pagination strategies returning a uniform `Page<T>`:

```rust
pub struct Page<T> {
    pub results: Vec<T>,
    pub has_more: bool,
    pub limit: usize,
    pub offset: Option<usize>,
    pub total_count: Option<usize>,
    pub after: Option<String>,
    pub before: Option<String>,
}
```

### Keyset Cursor-Based Pagination (`page_keyset`)
Best for high-volume datasets, feeds, and infinite scrolls. Supports custom sorting across multiple columns with deterministic tie-breaking and bidirectional navigation (`after` and `before`):

```rust
// First page (sorted by views descending):
let page1 = Article::query(&ctx)
    .sort_by(Article::views, true)
    .page_keyset(20, None, None)
    .await?;

// Next page forward (using after cursor):
let page2 = Article::query(&ctx)
    .sort_by(Article::views, true)
    .page_keyset(20, page1.after.as_deref(), None)
    .await?;

// Previous page backward (using before cursor, returns immediately preceding page in original order):
let prev_page = Article::query(&ctx)
    .sort_by(Article::views, true)
    .page_keyset(20, None, page2.before.as_deref())
    .await?;
```

### Offset Pagination (`page_offset`)
Best for traditional page-number navigation:

```rust
let page = Article::query(&ctx)
    .page_offset(10, 20, /* count_total */ true)
    .await?;

println!("Total articles: {:?}", page.total_count());
```

---

## 5. Field-Level Policies & Data Redaction

While resource-level `policies` authorize whole-record access, `field_policies` secure individual attributes:

```rust
resource! {
    resource Employee;
    table "employees";

    attributes {
        id: Uuid [pk],
        user_id: Uuid,
        name: String,
        ssn: Option<String>,
        salary: Option<i64>,
    }

    policies {
        policy authenticated {
            authorize_if actor_present;
        }
    }

    field_policies {
        // ssn visible only to the employee themselves or HR admins
        field ssn {
            authorize_if relates_to_actor(user_id);
            authorize_if actor_attribute_equals(role, "admin");
        }

        // salary visible only to admins
        field salary {
            authorize_if actor_attribute_equals(role, "admin");
        }
    }
}
```

### Behavior:
1. **Redaction on Read**:
   If an unauthorized actor queries or gets an `Employee`, unauthorized fields are redacted to `Value::Null` (`None`).
2. **Authorization on Write**:
   If an actor attempts to write or update a field without authorization, `Changeset` rejects the mutation with `Error::Forbidden`.
3. **Safe Partial Updates**:
   Updating non-sensitive fields does not inadvertently erase redacted sensitive fields.

---

## 6. PubSub & Action Notifiers

In the Ash architecture, **Notifiers** run side-effects (e.g. broadcasting events, sending emails, notifying webhooks) **after an action has successfully committed**. They are strictly decoupled from the primary database transaction and never block or invalidate persistence.

### Core Primitives:
- `Notification`: Comprehensive event payload containing `resource`, `action`, `action_kind` (`Create`, `Update`, `Destroy`), `id`, `record_fields`, `previous_fields`, `actor`, and `metadata`.
- `Notifier`: Asynchronous event consumer trait:
  ```rust
  pub trait Notifier: Send + Sync + Debug + 'static {
      fn notify<'a>(
          &'a self,
          notification: &'a Notification,
      ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;
  }
  ```
- `PubSub`: High-throughput, pattern-matching in-memory event broker:
  - Supports wildcard topics: `"orders:*"`, `"order:create"`, `"*"`, `"order:*:paid"`.
  - Deduplicated delivery: each matching subscription receives an event exactly once per publication, even when matching multiple topic aliases.
- `PubSubNotifier`: Built-in notifier that bridges resource notifications to `PubSub`.

### Example: PubSub Event Subscription

```rust
use std::sync::Arc;
use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_pubsub::{PubSub, PubSubNotifier};

// 1. Initialize PubSub broker and notifier from `ash-pubsub`
let pubsub = Arc::new(PubSub::new());
let notifier = Arc::new(PubSubNotifier::new(Arc::clone(&pubsub)));

// 2. Attach to execution Context (defined in `ash-core`)
let ctx = Context::new(Memory::new()).with_notifier(notifier);

// 3. Subscribe to topic patterns
let mut sub_all_orders = pubsub.subscribe("order:*");
let mut sub_order_creates = pubsub.subscribe("order:create");

// 4. Execute an action
let order = Order::create(&ctx)
    .customer("Alice")
    .amount(250)
    .await?;

// 5. Asynchronously receive typed notifications
let notif = sub_order_creates.recv().await?;
assert_eq!(notif.action, "create");
assert_eq!(notif.id, order.id);
```

### Transactional Atomicity with `Multi`:
When actions execute inside an atomic `Multi` transaction:
1. Notifications are **buffered in memory** while database mutations take place.
2. If any step fails and the transaction rolls back, **all queued notifications are dropped**—zero events are ever broadcast for uncommitted state.
3. If and only if the transaction commits successfully, all accumulated notifications are sequentially dispatched.

---

## 7. Ergonomic Leaps & Declarative Fluency

`ash-rust` provides declarative syntax improvements that eliminate boilerplate while preserving strict Rust compile-time type safety.

### 1. DRY Action Inputs (`accept [field1, field2]`)
Instead of repeating attribute types in action `accept` blocks, write `accept [field1, field2];`. The macro resolves types directly from `attributes`:

```rust
attributes {
    title: String,
    content: String,
    tag: Option<String>,
}

actions {
    create publish {
        // Types automatically resolved from attributes above!
        accept [title, content, tag];
    }
}
```

Optional field setters also implement `IntoOption`, accepting `&str`, `Some(...)`, and `None` without cumbersome wrapping:
```rust
Post::publish(&ctx)
    .title("Ergonomics")
    .content("Body")
    .tag("rust") // accepts &str directly!
    .await?;
```

### 2. Zero-Import Field Operators (`Resource::field`)
Attributes, calculations, aggregates, and relationships are automatically generated as associated constants directly on the resource struct. You no longer need to import `super::fields as f`:

```rust
// Query filtering with zero imports:
let active = Article::query(&ctx)
    .filter(Article::title.eq("Rust") & Article::views.gte(100))
    .load_rel(Article::comments)
    .load_aggregate(Article::comment_count)
    .all()
    .await?;
```

### 3. Typed PubSub Ergonomics
Fluently attach `PubSub` to contexts and subscribe to typed event streams without string topic typos:

```rust
use ash_pubsub::{ContextPubSubExt, PubSub, PubSubResourceExt};

// 1. Fluent context setup:
let ctx = Context::new(Memory::new()).with_pubsub(Arc::clone(&pubsub));

// 2. Typed subscription streams:
let mut sub_all = Article::subscribe_all(&pubsub);              // "article:*"
let mut sub_publish = Article::subscribe_action(&pubsub, "publish"); // "article:publish"
let mut sub_record = article.subscribe(&pubsub, Some("archive")); // "article:<id>:archive"
```

### 4. Record Lifecycle Helpers
Every `Resource` instance has access to `ResourceExt` helpers:

```rust
// Reload the latest state from the database:
let refreshed = article.reload(&ctx).await?;

// Destroy record using primary destroy action:
article.destroy(&ctx).await?;
```

### 5. Fluid Multi Pipeline (`ctx.multi()`)
Action builders directly implement `IntoChangeset`. You can pipe action builders directly into `ctx.multi()` without calling `.changeset()` or `.unwrap()`:

```rust
let results = ctx.multi()
    .create("author", Author::create(&ctx).name("Alice"))
    .create("post", Post::create(&ctx).title("First Post").content("Content"))
    .create_from("comment", |ctx, results| {
        let post: &Post = results.get("post").unwrap();
        Comment::create(ctx).post_id(post.id).body("Great post!").changeset()
    })
    .insert("batch_id", 42_i64)
    .commit() // runs atomically inside transaction when supported
    .await?;
```

---

## 8. Query Preparations

Preparations allow read actions to declare default query constraints (filters, sorting, limits, offsets) that run automatically every time the action is invoked.

```rust
resource! {
    resource Post;
    table "posts";

    attributes {
        id: Uuid [pk],
        title: String,
        status: String = "draft",
        views: i64 = 0,
        archived: bool = false,
    }

    actions {
        // Primary read filters out archived posts by default
        read read {
            primary;
            prepare filter(archived == false);
        }

        // Custom read action with default filter, descending sort, and limit
        read leaderboard {
            prepare filter(status == "published");
            prepare filter(archived == false);
            prepare sort(views, desc);
            prepare limit(10);
        }
    }
}
```

### Usage
```rust
// Invariant: Preparations apply automatically
let active_posts = Post::query(&ctx).all().await?; // archived == false is active!

// Custom action preparations apply seamlessly
let top_ten = Post::leaderboard(&ctx).all().await?;

// Additional user filters chain via AND
let top_rust_posts = Post::leaderboard(&ctx)
    .filter(Post::title.eq("Rust in 2026"))
    .all()
    .await?;
```

---

## 9. Custom & SQL-Executable Calculations

Calculations compute dynamic values from attributes or other expressions. Simple calculations compile directly to SQL in SQLite (`SELECT`, `WHERE`, `ORDER BY`), while custom calculations run through Rust functions in memory:

```rust
fn custom_discount(fields: &FieldMap) -> ash_core::Result<Value> {
    let price = fields.get("price").and_then(|v| v.as_int()).unwrap_or(0);
    Ok(Value::String(if price > 50 { "VIP".into() } else { "NORMAL".into() }))
}

resource! {
    resource Product;
    table "products";

    attributes {
        id: Uuid [pk],
        name: String,
        code: String,
        price: i64,
        quantity: i64,
        views: i64 = 0,
        nickname: Option<String>,
    }

    calculations {
        // Arithmetic expressions:
        total_price: i64 = price * quantity;
        bonus_views: i64 = views + 100;

        // Built-in string/SQL functions:
        name_length: i64 = length(name);
        upper_code: String = upper(code);
        display_name: String = coalesce(nickname, name, "Product");

        // Conditional branching:
        badge: String = if_else(views >= 500, "Popular", "Regular");

        // Custom Rust function calculation:
        discount_tier: String = custom(custom_discount);
    }
}
```

### Querying, Filtering, & Sorting with Calculations
```rust
let products = Product::query(&ctx)
    .calc(Product::total_price)
    .calc(Product::badge)
    .filter(Product::total_price.gt(500))  // compiles to WHERE (price * quantity) > 500
    .sort_desc(Product::total_price)       // compiles to ORDER BY (price * quantity) DESC
    .all()
    .await?;

assert_eq!(products[0].total_price, Some(2000));
assert_eq!(products[0].badge, Some("Popular".into()));
```

---

## 10. Multi-Store Context Routing (`DataLayerRegistry`)

Declare the target data layer per resource directly in the DSL, and let a single `Context` route operations across multiple backends:

```rust
resource! {
    resource User;
    table "users";
    data_layer sqlite; // persistent SQL store

    attributes {
        id: Uuid [pk],
        email: String,
    }
}

resource! {
    resource Session;
    table "sessions";
    data_layer memory; // ephemeral in-memory cache

    attributes {
        id: Uuid [pk],
        token: String,
        user_id: Uuid,
    }
}
```

### Context Setup & Multi-Store Transactions
```rust
use ash_core::{Context, DataLayerKind, DataLayerRegistry};

// 1. Configure registry
let registry = DataLayerRegistry::new()
    .register_kind(DataLayerKind::Sqlite, sqlite)
    .register_kind(DataLayerKind::Memory, memory);

let ctx = Context::new(registry);

// 2. Operations automatically route to the right store!
let user = User::create(&ctx).email("alice@example.com").await?;    // writes to SQLite
let session = Session::create(&ctx).token("tok_123").user_id(user.id).await?; // writes to Memory

// 3. Multi pipeline crossing stores seamlessly
ctx.multi()
    .create("user", User::create(&ctx).email("bob@example.com"))
    .create_from("session", |ctx, res| {
        let u: &User = res.get("user").unwrap();
        Session::create(ctx).token("tok_bob").user_id(u.id).changeset()
    })
    .commit()
    .await?;
```

---

## 12. Native Rust Enum Types (`#[derive(AshEnum)]` / `AshType`)

Eliminates stringly-typed attributes for status, category, and state columns. Native enums compile down to strings for SQL / in-memory storage, while giving callers 100% compile-time type safety, exhaustive pattern matching, and typed filter operators.

### Defining an Enum
```rust
use ash_core::AshEnum;

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum TicketStatus {
    Draft,                       // serializes to "draft" by default
    #[ash(string = "in_progress")]
    InProgress,                  // custom serialized name
    #[ash(rename = "resolved")]
    Resolved,
    Closed,
}
```

### Resource Definition
Mark attributes with `[enum]` or `[enum, default: ...]`:
```rust
resource! {
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        subject: String,
        status: TicketStatus [enum, default: TicketStatus::Draft],
        priority: Option<Priority> [enum],
    }

    actions {
        create create {
            accept [subject, status, priority];
        }
    }
}
```

### Type-Safe Queries & Matching
```rust
// Create with typed enum:
let ticket = Ticket::create(&ctx)
    .subject("Network Issue")
    .status(TicketStatus::InProgress)
    .await?;

// Exhaustive pattern matching without string parsing or wildcard fallback:
match ticket.status {
    TicketStatus::Draft => println!("Draft"),
    TicketStatus::InProgress => println!("Active"),
    TicketStatus::Resolved => println!("Done"),
    TicketStatus::Closed => println!("Archived"),
}

// Type-safe filter expressions:
let active = Ticket::query(&ctx)
    .filter(Ticket::status.eq(TicketStatus::InProgress))
    .all()
    .await?;
```

---

## 13. Extended Policy Primitives (`bypass`, `forbid_if`, `authorize_unless`, `forbid_unless`)

Ash policies support expressive negative conditions and bypass blocks that allow privileged actors to skip authorization ladders completely.

### Bypass Policies
Bypass blocks run first. If a bypass condition succeeds (such as an admin actor check), all subsequent policies are bypassed and the request is immediately authorized:

```rust
policies {
    // Admins skip all subsequent checks unconditionally
    bypass {
        authorize_if actor_attribute_equals(role, "admin");
    }

    // Normal policy ladder
    policy {
        forbid_if actor_attribute_equals(status, "banned");
        authorize_if relates_to_actor(author_id);
    }
}
```

### Expressive Negative Primitives
- `forbid_if(check)`: Immediately denies authorization if the check evaluates to `true`.
- `forbid_unless(check)`: Immediately denies authorization if the check evaluates to `false`.
- `authorize_unless(check)`: Authorizes if the check evaluates to `false`.
- `authorize_if(check)`: Authorizes if the check evaluates to `true`.

Applicable across both resource policies and `field_policies` for fine-grained column redaction and field write controls.

---

## 14. Changeset Lifecycle Hooks (`before_action`, `after_action`, `after_transaction`)

Ash changesets support granular lifecycle closure hooks directly on `Changeset<R>` and on action invocation builders:

- **`before_action`**: Executes right before data layer persistence. Receives mutable access to `&mut Changeset<R>`, allowing attribute modifications (`cs.change_attribute(...)`, sanitization, slug computation) or aborting the action by returning `Err(Error)`.
- **`after_action`**: Executes immediately after persistence within the transaction. Receives mutable access to the newly saved record `&mut R` (for audit logging, child associations, or in-memory enrichment).
- **`after_transaction`**: Executes after transaction completion (commit or rollback), receiving `Result<&R, &Error>`.

### Usage on Action Builders
Hooks can be chained directly onto fluent action builders:

```rust
let article = Article::create(&ctx)
    .title("Ash Framework in Rust")
    .body("Ad-hoc lifecycle hooks")
    // 1. Run right before database write
    .before_action(|cs| {
        if let Some(Value::String(title)) = cs.get_attribute("title") {
            let slug = title.to_lowercase().replace(' ', "-");
            cs.change_attribute("slug", slug);
        }
        Ok(())
    })
    // 2. Run right after persistence
    .after_action(|record| {
        println!("Persisted article with ID: {}", record.id);
        Ok(())
    })
    // 3. Run after transaction commit or rollback
    .after_transaction(|res| {
        match res {
            Ok(record) => println!("Transaction committed: {}", record.id),
            Err(err) => eprintln!("Transaction rolled back: {err}"),
        }
    })
    .await?;
```

### Usage on `Changeset<R>` Directly
Hooks can also be attached when constructing changesets manually or inside `Multi` pipelines:

```rust
let mut cs = Article::create(&ctx).title("Manual CS").changeset()?;
cs = cs.before_action(|cs| {
    cs.change_attribute("view_count", 0);
    Ok(())
});
let article = cs.commit(&ctx).await?;
```

---

## 15. Cascading Relationship Deletes (`on_delete: cascade | nilify | restrict`)

Relationships declare delete behavior. When a parent entity is destroyed via an Ash destroy action, Ash executes application-layer cascading actions for all dependent child records (ensuring child policies, validations, and notifiers are triggered rather than solely relying on raw database foreign key cascades):

- **`cascade`**: Recursively finds and destroys all matching child records using their primary destroy actions. Also cascades across join resources in `many_to_many` relationships.
- **`nilify`**: Updates all dependent child records to set their foreign key attribute to `null`.
- **`restrict`**: If any dependent records exist when attempting to delete the parent, immediately aborts with `Error::DeleteRestricted { resource, relationship, count }`.

### Resource Definition
```rust
resource! {
    resource Author;
    table "authors";

    attributes {
        id: Uuid [pk],
        name: String,
    }

    relationships {
        // Multi-level recursive cascade
        has_many posts: Post [fk: "author_id", on_delete: cascade],
    }

    actions {
        create create { accept [name]; }
        destroy destroy { primary; }
    }
}

resource! {
    resource Department;
    table "departments";

    attributes {
        id: Uuid [pk],
        name: String,
    }

    relationships {
        // Aborts parent deletion if employees still exist
        has_many employees: Employee [fk: "dept_id", on_delete: restrict],
    }

    actions {
        create create { accept [name]; }
        destroy destroy { primary; }
    }
}

resource! {
    resource Team;
    table "teams";

    attributes {
        id: Uuid [pk],
        name: String,
    }

    relationships {
        // Sets player.team_id = null when team is deleted
        has_many players: Player [fk: "team_id", on_delete: nilify],
    }

    actions {
        create create { accept [name]; }
        destroy destroy { primary; }
    }
}
```

---

## 16. Managed Relationships (`manage_relationship` / Nested Writes)

Ash allows executing nested writes directly inside create and update actions without manually orchestrating multi-step pipelines. Managed relationships handle inserting children, populating foreign keys, updating existing records, and removing/disassociating omitted records atomically within the action transaction.

### Mutation Modes (`ManagedRelType`)
- **`Create`**: Creates child records with their foreign key pointing to the parent.
- **`Append`**: Adds new child records or links existing ones without removing omitted records.
- **`DirectControl`**: Full synchronization. Matches child inputs against existing records by primary key (`id`):
  - Items matching existing records are updated.
  - Items without an `id` (or with a new `id`) are created.
  - Existing records for this parent omitted from the inputs are deleted (or foreign keys set to `null` if the relationship specifies `on_delete: nilify`).

### Usage on Fluent Action Builders
Action builders generate strongly typed `manage_<rel>` methods as well as generic `.manage_relationship(...)` and `.manage_relationship_one(...)`:

```rust
// 1. Create Order with nested LineItems
let order = Order::create(&ctx)
    .customer("Alice")
    .manage_items(
        vec![
            LineItem::create(&ctx).item("Book").price(25),
            LineItem::create(&ctx).item("Pen").price(5),
        ],
        ManagedRelType::Create,
    )
    .await?;

// 2. Synchronize existing Order via DirectControl
let updated_order = Order::update(&ctx, order.id)
    .customer("Alice Cooper")
    .manage_items(
        vec![
            // Keep and update existing item
            [("id", Value::Uuid(item1.id)), ("price", Value::Int(30))].into_field_map(),
            // Add new item
            LineItem::create(&ctx).item("Bookmark").price(2).into_fields(),
            // Omitted items are destroyed automatically!
        ],
        ManagedRelType::DirectControl,
    )
    .await?;

// 3. Create parent with belongs_to child
let user = User::create(&ctx)
    .username("will")
    .manage_relationship_one(
        "profile",
        Profile::create(&ctx).bio("Software Engineer"),
        ManagedRelType::Create,
    )
    .await?;
```

### Declarative DSL via `change manage_relationship(...)`
Actions can accept arguments and declaratively manage the relationship:

```rust
resource! {
    resource Order;
    table "orders";

    attributes {
        id: Uuid [pk],
        customer: String,
    }

    relationships {
        has_many items: LineItem [fk: "order_id"],
    }

    actions {
        create create_with_items {
            primary;
            accept [customer];
            argument items: Vec<FieldMap>;
            change manage_relationship(items, create);
        }
    }
}
```

## 17. Bulk & Batch Operations (`bulk_create`, `bulk_destroy`, chunked streaming)

`ash-rust` provides native, optimized bulk data operations directly mirroring Ash Elixir's bulk actions:
- `Resource::bulk_create(&ctx, inputs)`: Validates, sets defaults, checks policies, and inserts records in batched chunks.
- `Resource::bulk_destroy(&ctx, ids)`: Verifies authorizations, applies cascading relationship deletes, and removes records using optimized multi-id batch deletes (`WHERE id IN (...)`).
- `Query::bulk_destroy(&ctx, "destroy", opts)`: Bulk destroys all records matching any complex query filter.
- `Query::chunked(batch_size, |chunk| ...)`: Streams query results in chunks without exhausting memory.
- `Query::chunked_keyset(batch_size, |chunk| ...)`: Streams keyset-ordered queries across massive tables with constant performance.
- `Multi::bulk_create` & `Multi::bulk_destroy`: Atomic bulk operations inside transactional multi pipelines.

### Bulk Options (`BulkCreateOptions`, `BulkDestroyOptions`)
- `batch_size(usize)`: Chunk large collections into batches of N rows (default: all at once).
- `return_records(bool)`: Controls whether loaded/redacted records are returned in `BulkResult`.
- `stop_on_error(bool)`: Whether to halt on the first validation/persistence error or collect errors in `BulkResult.errors`.
- `notify(bool)`: Emits lifecycle action notifications to registered notifiers for each record.
- `upsert(identity, update_fields)`: Runs atomic upsert operations on conflict with the specified identity constraint.

### Example Usage
```rust
// 1. Bulk Create with chunking
let items = (1..=1000).map(|i| {
    [
        ("sku", format!("SKU-{i}").into()),
        ("title", format!("Product {i}").into()),
        ("price", 100.into()),
    ]
});

let opts = BulkCreateOptions::new().batch_size(250);
let result = Product::bulk_create_with_opts(&ctx, "create", items, opts).await?;
println!("Inserted {} items with {} errors", result.count, result.error_count);

// 2. Query Bulk Destroy
let del_result = Product::query(&ctx)
    .filter(Product::category.eq("Archived"))
    .bulk_destroy("destroy", BulkDestroyOptions::default())
    .await?;

// 3. Chunked Streaming
Product::query(&ctx)
    .filter(Product::price.gt(50))
    .chunked(100, |chunk| async move {
        for product in chunk {
            process(product);
        }
        Ok(())
    })
    .await?;
```






