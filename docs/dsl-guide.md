# DSL & Modeling Guide

The `resource!` and `domain!` procedural macros in `ash-macros` provide an expressive, declarative domain-specific language for modeling resources, relationships, validations, and operations. `define!` is an alias of `resource!`.

Canonical header is `Name { ... }`. Put `embedded` before the name when the resource is not persisted as its own table. Bind storage with `store Type;`.

Items end with `;`. Inside `[...]` lists, commas separate names and a trailing comma is allowed. Empty sections may be `{}` or omitted. Required create/generic inputs are typestate-gated: omitting them is a compile error.

| Write | Don't write |
| :--- | :--- |
| `Post { ... }` | `resource Post;` / `name Post;` |
| `embedded Address { ... }` | `embedded;` in the body |
| `Blog { ... }` in `domain!` | `domain Blog;` |
| `Ticket;` in `resources` | `resource Ticket` |
| `primary;` | `primary true;` |
| `min: 3` | `min = 3` or positional `string_length(title, 3)` |
| `store SqliteStore;` | `data_layer sqlite;` |
| `change set(status = ...)` | `change set_attribute(...)` |
| `validate present(title);` | `validation present(title);` |
| `prepare filter(...)` | `preparation filter(...)` |
| `generic ping { ... }` | `action ping { ... }` |
| `define close action: close on: record;` | commas after the name or between options |
| `fk: author_id` | `fk: "author_id"` |
| `has_many comments: Comment` | `has_many comments: Vec<Comment>` |
| `belongs_to author: Author` / `has_one profile: Profile` | `Option<Author>` / `Option<Profile>` |

---

## 1. Defining Resources (`resource!`)

```rust
use ash_core::{resource, AshEnum};
use uuid::Uuid;

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum PostStatus {
    Draft,
    Published,
}

resource! {
    Post {
        table "posts";
        store PrimaryDb;

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            title: String;
            content: Option<String>;
            views: i64;
            status: PostStatus [enum, default: PostStatus::Draft];
            version: i64 [version];
        }

        relationships {
            belongs_to author: Author [fk: author_id];
            has_one cover: Cover [fk: post_id];
            has_many comments: Comment [fk: post_id];
            many_to_many tags: Tag [through: PostTag, source_fk: post_id, dest_fk: tag_id];
        }

        calculations {
            title_length: Option<i64> = string_length(title);
            total_price: Option<i64> = price * quantity;
            display_label: Option<String> = coalesce(nickname, title, "Untitled");
            badge: Option<String> = if_else(views >= 1000, "Trending", "Normal");
            custom_rank: Option<String> = custom(my_rank_calculator);
        }

        aggregates {
            comment_count: Option<i64> = count(comments);
            has_tags: Option<bool> = exists(tags);
        }

        policies {
            policy action_type(read) {
                authorize_if always;
            }
            policy action(create) | action(publish) | action(destroy) {
                authorize_if actor_eq(role = "author");
            }
        }

        field_policies {
            field content {
                authorize_if actor_present;
            }
        }

        notifiers [
            &AUDIT_NOTIFIER
        ]

        actions {
            create create {
                primary;
                accept [title, content];
                change set(views = 0);
                validate present(title);
                validate string_length(title, min: 3, max: 255);
            }

            read read {
                primary;
                prepare filter(archived == false);
            }

            read trending {
                prepare filter(views >= 100);
                prepare sort(views, desc);
                prepare limit(10);
            }

            update publish {
                argument notify_subscribers: bool;
                change set(status = PostStatus::Published);
            }

            destroy destroy {
                primary;
            }
        }
    }
}
```

If any `policies { ... }` block is present, every action must be covered by `policy always`, `policy action(name)`, or `policy action_type(kind)`. Uncovered actions are compile errors.

`prepare` is only valid on `read`. `accept` / `change` / `validate` are not valid on `read`. `persist manual` is only valid on `create`. `returns` / `run` are only valid on `generic`. `relate_actor` is not valid on `generic`. Those errors underline the illegal keyword, not the action name.

Typos get a "Did you mean?" plus the names that belong in that slot, including `prepare filter(archved == false)`. Using a relationship or calculation where an attribute is required is a distinct error (`accept [comments]` says `comments` is a relationship). `prepare filter(comments == ...)` is the same kind of wrong-slot error.

`validate present(title)` on a non-`Option` field, `accept [status]` together with `change set(status = ...)`, and unused `argument`s are rustc **warnings** (via `#[deprecated]`), not errors.

IDE completion inside `accept [...]` (including empty `[]`), `policy action(...)`, aggregates, and identity keys is scoped to the names valid in that slot. `string_length` / `length` / `lower` / `upper` calculations require a string field; arithmetic and string ops are type-checked against the declared calculation type (`Option<T>` may wrap `T`). Action builders are `#[must_use]` until `.await` / `.call()`. A missing `StoreTag` / `HasStore<T>` bound names the store to register.

---

## 2. Attributes & Modifiers

Attributes represent persisted state or fields on the underlying data layer. Each attribute ends with `;`.

| Syntax | Description |
| :--- | :--- |
| `id: Uuid [pk];` | Primary key for the resource (`[pk]` also marks it generated) |
| `field: String;` | Non-nullable string attribute |
| `field: Option<i64>;` | Nullable integer attribute |
| `priority: i32;` | Integer attribute (`i8`/`u32`/… store as `Integer`) |
| `version: i64 [version];` | Optimistic locking concurrency attribute |
| `status: String = "draft";` | Default static value on create |
| `views: i64 [default: 0];` | Bracket syntax for defaults |
| `token: String [default_fn: gen_token];` | Dynamic default generated by a function |
| `status: PostStatus;` | Typed enum via `#[derive(AshEnum)]`. `[enum]` is optional |

Do not use `[atom: "a,b"]`. Model closed sets as a Rust enum with `#[derive(AshEnum)]`. `[enum]` is optional when the field type already implements `AshEnum`; `[enum]` on a builtin scalar is an error. `one_of` is invalid on enum fields — variants already constrain the value.

### Timestamps Shorthand

Add `timestamps;` to inject `created_at: String` and `updated_at: String` ISO-8601 attributes. Rename them with `timestamps [inserted_at, modified_at];`. Either form may sit in the resource body or inside `attributes { ... }`.

```rust
resource! {
    Article {
        table "articles";
        timestamps;

        attributes {
            id: Uuid [pk];
            title: String;
        }
    }
}
```

---

## 2b. Identities & Unique Constraints

```rust
resource! {
    User {
        table "users";

        attributes {
            id: Uuid [pk];
            organization_id: Uuid;
            email: String;
        }

        identities {
            identity unique_email: [email], message: "Email is already taken";
            identity org_email: [organization_id, email];
        }
    }
}
```

### Auto-Generated Identity Lookups & Upserts

```rust
let user = User::get_by_unique_email(&ctx, "alice@example.com").await?;
let user_opt = User::find_by_unique_email(&ctx, "alice@example.com").await?;
let user = User::get_by_org_email(&ctx, org_id, "alice@example.com").await?;

let user = User::create(&ctx)
    .email("alice@example.com")
    .upsert_on(User::unique_email, &["name"])
    .call()
    .await?;
```

---

## 2c. Embedded Resources

```rust
resource! {
    embedded Address {
        attributes {
            street: String;
            city: String;
            postal_code: String;
        }

        actions {
            create create {
                primary;
                accept [street, city, postal_code];
                validate present(street);
            }
        }
    }
}
```

---

## 3. Relationships

Foreign keys are identifiers, not strings. Destination types are the related resource. Wrapping is inferred: `belongs_to` / `has_one` become `Rel<Option<Dest>>`; `has_many` / `many_to_many` become `Rel<Vec<Dest>>`. Writing `Option<Dest>` or `Vec<Dest>` in the DSL is an error.

`fk` is optional. `belongs_to` defaults to `{rel}_id`. `has_one` and `has_many` default to `{parent_resource}_id` on the destination.

### `belongs_to`

```rust
belongs_to author: Author [fk: author_id];
```

### `has_one`

```rust
has_one profile: Profile [fk: user_id, on_delete: cascade];
```

### `has_many`

```rust
has_many comments: Comment [fk: post_id];
```

### `many_to_many`

```rust
many_to_many tags: Tag [through: PostTag, source_fk: post_id, dest_fk: tag_id];
```

`on_delete` is `cascade`, `nilify`, `restrict`, or `nothing` (identifiers, not `"cascade"` strings). `source_attribute_on_join_resource` / `destination_attribute_on_join_resource` are aliases of `source_fk` / `dest_fk`.

---

## 4. Calculations & Aggregates

### Calculations

```rust
calculations {
    title_length: Option<i64> = string_length(title);
    total: i64 = price * quantity;
    discounted(discount: i64): i64 = price - arg(discount);
}
```

`string_length(title)` is a compile error if `title` is not a string. The struct field is always `Option<inner>`: declaring `i64` or `Option<i64>` both yield `Option<i64>` on the resource.

Calculation arguments use `(name: Type)` after the calculation name and `arg(name)` in the expression. Load them with `query.calc_with_args(Item::discounted, args)`.

Built-ins: `string_length`, `length`, `concat`, `coalesce`, `lower`, `upper`, `if_else`, `custom`, arithmetic, comparisons, and `null`. Quoted `"string_length(title)"` is an error.

### Aggregates

- `count(relationship)`
- `exists(relationship)`
- `sum(relationship, field)`
- `first(relationship, field)`

```rust
aggregates {
    tag_count: Option<i64> = count(tags);
    has_comments: Option<bool> = exists(comments);
    open_ticket_count: Option<i64> = count(tickets, filter: status == "open");
}
```

---

## 5. Actions, Arguments, Validations, and Changes

### Action Kinds

- `create`: Inserts a new record. `persist manual` skips the data layer; the builder exposes `.persist(|ctx, record| async { ... })` instead of writing through the store.
- `read`: Queries records. May use `prepare filter(...)` / `sort` / `limit` / `offset`.
- `update`: Mutates an existing record.
- `destroy`: Deletes a record.
- `generic`: Custom logic. Typed `accept { name: Type }` is allowed here only. Return type is `generic name, Type { ... }` or `returns Type;` inside the body. May omit `run` when the caller supplies `.run(...)`.

### Inputs

Create/update/destroy use DRY lists; types come from attributes. `read` cannot `accept`, `change`, or `validate`.

```rust
actions {
    create create {
        primary;
        accept [title, content];
        change set(status = PostStatus::Draft);
    }

    update publish {
        accept [tag];
        argument notify_subscribers: bool;
        change set(status = PostStatus::Published);
    }
}
```

Required create accept fields and non-`Option` arguments are typestate-required. Update accept fields are never required (partial updates). Optional (`Option<T>`) setters accept a bare value, `Some(...)`, or `None`.

`primary;` is a flag with no boolean. Write `min: 3`, not `min = 3` or positional integers.

`persist manual` is create-only. The generated builder does not write to the store; you supply persistence:

```rust
create intake {
    accept [subject];
    persist manual;
}

let ticket = Ticket::intake(&ctx)
    .subject("Printer jammed")
    .persist(|_ctx, ticket| async move {
        intake_store().lock().unwrap().insert(ticket.id, ticket.clone());
        Ok(ticket)
    })
    .await?;
```

### Built-in Validations

- `validate present(field);`
- `validate string_length(field, min: X, max: Y);`
- `validate numericality(field, min: X, max: Y);`
- `validate one_of(field, ["opt1", "opt2"]);` — not on enum attributes
- `validate custom(&MyValidator);` — `&'static dyn CustomValidation`
- `validate func(|ctx| { ... });`

### Built-in Changes

`set(...)` is real Rust. Literals become `Change::SetAttribute`; paths/enums become `SetAttributeFn`.

- `change set(field = value);`
- `change set_new(field = value);`
- `change relate_actor(field);`
- `change set_from_arg(field, argument_name);` or `change set(field = arg(name));`
- `change manage_relationship(rel);` or `change manage_relationship(rel, create);` / `manage_relationship(rel, type: create)`
- `change func(my_fn);` — `fn(&mut ChangeContext<'_>) -> Result<()>`
- `change custom(&MyChange);` — `&'static dyn CustomChange`
- `change before_action(my_fn);` / `before_action my_fn;` — `BeforeActionFn`

### Policies

Every check ends with `;`. `policy action(name)` is go-to-definition on the action. `actor_eq` / `actor_attribute_equals` require an `actor { field: Type; }` block on the resource. `relates_to` / `relates_to_actor` complete relationship or attribute names; `is_nil` and `eq` complete attributes and type-check the comparison value.

```rust
actor {
    role: String;
}

policies {
    policy action_type(read) {
        authorize_if always;
    }
    policy action(create) {
        authorize_if actor_eq(role = "admin");
    }
}
```

### Notifiers Block

```rust
notifiers [
    &AUDIT_LOG_NOTIFIER,
    &METRICS_NOTIFIER
]
```

Alternatively, attach notifiers per-request via `ctx.with_notifier(...)`. Each entry is type-checked as `&'static dyn Notifier`. `extensions [&STATE_MACHINE]` is `&'static dyn ResourceExtension`. `optimistic_lock version;` must name an attribute (same effect as `[version]` on that attribute). Aggregate `filter:` fields are checked on the related resource.

---

## 6. Generated Ergonomic APIs

```rust
let post = Post::create(&ctx)
    .title("My Post")
    .content("Hello World")
    .await?;

let posts = Post::query(&ctx)
    .filter(Post::views.gt(10) & Post::archived.eq(false))
    .load_rel(Post::tags)
    .load_aggregate(Post::tag_count)
    .all()
    .await?;

let reloaded = post.reload(&ctx).await?;
post.destroy(&ctx).await?;

let results = ctx.multi()
    .create("author", Author::create(&ctx).name("Alice"))
    .create("post", Post::create(&ctx).title("First Post").content("Content"))
    .commit()
    .await?;
```

---

## 7. Defining Domains (`domain!`)

Canonical header is `Name { ... }`, matching `resource!`. Resource entries and code interfaces end with `;`. Code interface options are space-separated (`define open_ticket action: open`), not comma-separated. `action: open` probes `Ticket::open` so F12 goes to the action. If the `domain!` body is incomplete, a first-pass token walk still type-checks resource names and `action:` values so rust-analyzer can complete them.

Options on `define`:

- `action: name` (required)
- `args: [name: Type, ...]`
- `get_by: id` for get-by-primary-key
- `on: record` or `on: id` for update/destroy-style interfaces that take a record or id

```rust
use ash_core::domain;

domain! {
    Blog {
        resources {
            Post {
                define create_post action: create args: [title: String];
                define get_post action: read get_by: id;
                define publish_post action: publish on: record;
            };
            Author;
            Tag;
        }
    }
}

let blog = Blog::new(Sqlite::connect("sqlite://blog.db").await?);
blog.install().await?;
let posts = blog.query::<Post>().all().await?;
```

---

## 8. Bulk Operations & Streaming

```rust
let items = vec![
    [("title", "Post 1".into()), ("content", "...".into())],
    [("title", "Post 2".into()), ("content", "...".into())],
];
let res = Post::bulk_create(&ctx, items).await?;

let del_res = Post::query(&ctx)
    .filter(Post::archived.eq(true))
    .bulk_destroy("destroy", BulkDestroyOptions::default())
    .await?;

Post::query(&ctx)
    .chunked(100, |batch| async move {
        Ok(())
    })
    .await?;
```

---

## 9. Declarative Action Hooks

```rust
fn normalize_title(fields: &mut FieldMap) -> Result<()> {
    if let Some(Value::String(s)) = fields.get("title") {
        fields.insert("title".into(), Value::String(s.trim().to_string()));
    }
    Ok(())
}

fn emit_metrics(res: std::result::Result<&FieldMap, &Error>) {
    if res.is_ok() {
        println!("Action committed successfully");
    }
}

fn audit_logger(ctx: &mut ChangeContext<'_>) -> Result<()> {
    ctx.after_action(|fields| {
        println!("Saved record: {:?}", fields.get("id"));
        Ok(())
    });
    Ok(())
}

struct AuditLogger;
impl CustomChange for AuditLogger {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        ctx.after_transaction(|res| {
            if res.is_ok() { /* log commit */ }
        });
        Ok(())
    }
}

resource! {
    Article {
        table "articles";

        attributes {
            id: Uuid [pk];
            title: String;
            body: String;
        }

        actions {
            create publish {
                primary;
                accept [title, body];

                before_action normalize_title;
                after_action |fields| {
                    println!("Saved article: {:?}", fields.get("title"));
                    Ok(())
                };
                after_transaction emit_metrics;

                change before_action(normalize_title);
                change func(audit_logger);
                change custom(&AuditLogger);
            }

            destroy archive {
                primary;
                before_action |fields| {
                    Ok(())
                };
            }
        }
    }
}
```

### Runtime Call-Site Hooks

```rust
let article = Article::create(&ctx)
    .title("Hello World")
    .body("...")
    .after_action(move |record| {
        telemetry.record("article_created", record.id, request_id);
        Ok(())
    })
    .after_transaction(move |res| {
        if res.is_ok() {
            metrics.increment("articles.published");
        }
    })
    .await?;
```

---

## 10. Generic Actions

Return type is either `generic name, Type { ... }` or `returns Type;` inside the body. Typed `accept { name: Type }` is for generic actions only. Every item, including `run`, ends with `;`.

```rust
resource! {
    CommunicationService {
        actions {
            generic send_notification {
                argument recipient: String;
                argument body: String;
                argument priority: Option<String>;
                returns String;

                run |input| async move {
                    let priority = input.priority.unwrap_or_else(|| "normal".into());
                    let sender = input.actor().map(|a| a.id.to_string()).unwrap_or_else(|| "system".into());
                    Ok(format!("{}: [{}] sent '{}' to {}", sender, priority, input.body, input.recipient))
                };
            }

            generic analyze_subject {
                accept {
                    text: String,
                };
                returns Analysis;
                run |input| async move {
                    Ok(Analysis {
                        word_count: input.text.split_whitespace().count(),
                        urgent: input.text.contains('!'),
                    })
                };
            }
        }
    }
}

let response = CommunicationService::send_notification(&ctx)
    .recipient("alice@example.com")
    .body("Hello, World!")
    .priority("high")
    .await?;
```

---

## 11. Type-Safe Store Tagging (`store <Type>;`)

```rust
pub struct PrimaryDb;
impl StoreTag for PrimaryDb {}

pub struct AuditDb;
impl StoreTag for AuditDb {}

resource! {
    Order {
        table "orders";
        store PrimaryDb;
        // ...
    }
}

resource! {
    AuditEvent {
        table "audit_events";
        store AuditDb;
        // ...
    }
}

let registry = StoreRegistry::new()
    .with_store::<PrimaryDb, _>(primary_sqlite)
    .with_store::<AuditDb, _>(audit_sqlite);

let ctx = Context::new(registry);
```

`store SqliteStore` / `MemoryStore` / `PostgresStore` also set `ResourceDef.data_layer`. Do not write `data_layer sqlite`.

---

## 12. Tenant & Request Metadata (`Context`, `ValidationContext`, `ChangeContext`)

Attribute multitenancy stamps and scopes records from `ctx.tenant()`. Create without a tenant returns `Error::TenantRequired`.

```rust
resource! {
    TenantDoc {
        table "tenant_docs";

        multitenancy {
            strategy: attribute;
            attribute: tenant_id;
        }

        attributes {
            id: Uuid [pk];
            tenant_id: String;
            title: String;
        }

        actions {
            create create {
                primary;
                accept [title];
            }
            read read { primary; }
        }
    }
}
```

`strategy: attribute` (the default if omitted) stamps and filters `attribute` from `ctx.tenant()`. `strategy: context` requires a tenant on the context without writing a column, which pairs with schema/`search_path` tenancy in `ash-postgres`. `global: true` skips the tenant requirement. `attribute` may be an identifier or a string.

```rust
let ctx = Context::new(data_layer)
    .with_tenant("tenant_acme")
    .with_metadata("trace_id", "trace-12345");

let post = Post::create(&ctx)
    .title("Tenant Post")
    .await?;

let alt_posts = Post::query(&ctx).tenant("tenant_beta").all().await?;

pub struct CheckTenant;
impl CustomValidation for CheckTenant {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()> {
        if let Some(tenant) = ctx.tenant {
            // validate tenant constraints
        }
        Ok(())
    }
}
```
