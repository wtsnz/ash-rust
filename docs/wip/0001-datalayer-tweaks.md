# RFC 0001: Type-Safe Data Layer Stores & Multi-Database Routing

- **Status**: Implemented
- **Date**: 2026-09-02
- **Authors**: Will, Ash-Rust Team
- **Target Crates**: `ash-core`, `ash-macros`

---

## 1. Summary

This RFC proposes evolving `ash-rust`'s data layer binding model from the current closed enum (`DataLayerKind::Sqlite`, `DataLayerKind::Memory`, etc.) to an open, **type-safe store marker system** (`StoreTag` + `StoreRegistry`).

This allows:
1. **Multiple instances of the same database engine** (e.g. `PrimarySqlite` and `AnalyticsSqlite`, or primary vs. read-replica) with static differentiation.
2. **First-class third-party data layers** (e.g. `ash-clickhouse`, `ash-postgres`, `ash-redis`) with **zero** changes to `ash-core`.
3. **Optional compile-time verification** that all stores required by resources are provided by the execution `Context`.
4. **Preserved storage-agnostic defaults** so unit tests can still run entirely in-memory without changing resource definitions.

---

## 2. Motivation & Background

### Current State in `ash-rust`

Currently, `ash-core` defines:
```rust
pub enum DataLayerKind {
    Memory,
    Sqlite,
    Embedded,
    Custom(&'static str),
}
```

Resources declare their data layer optionally:
```rust
resource! {
    resource User;
    data_layer sqlite;
}
```

And `DataLayerRegistry` routes operations based on this enum:
```rust
let registry = DataLayerRegistry::new()
    .register_kind(DataLayerKind::Sqlite, sqlite)
    .register_kind(DataLayerKind::Memory, memory);
```

### Problems with the Current Approach

1. **Closed Enum**: `DataLayerKind` is closed. A new crate like `ash-clickhouse` cannot add a variant to `DataLayerKind`. It is forced to use `DataLayerKind::Custom("clickhouse")`, which is stringly-typed, prone to typos, and lacks compiler support.
2. **Cannot Distinguish Multiple Databases of the Same Kind**: If an application has a `primary.db` SQLite database and an `events.db` SQLite database, both are `DataLayerKind::Sqlite`. The registry cannot distinguish between them without ad-hoc string-based resource name overrides.
3. **Comparison to Ash Elixir**: In Elixir, Ash separates the **adapter** (`AshPostgres.DataLayer` / `AshSqlite.DataLayer`) from the **Repo** (`MyApp.PrimaryRepo` vs. `MyApp.AnalyticsRepo`). In Elixir, repos are OTP processes configured in application environment files. In Rust, we need a solution that is idiomatic to Rust's type system, avoids global mutable state, and provides compile-time guarantees.

---

## 3. Detailed Design

### 3.1 The `StoreTag` Marker Trait

We define a zero-sized marker trait in `ash-core`:

```rust
// in ash-core/src/store.rs

/// A marker trait identifying a logical database or storage target.
pub trait StoreTag: 'static + Send + Sync {}

/// The default store marker used when a resource does not specify an explicit store.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DefaultStore;
impl StoreTag for DefaultStore {}
```

Applications or library crates define their database identities as regular Rust structs:

```rust
// in app/src/stores.rs
pub struct PrimaryDb;
pub struct AnalyticsDb;
pub struct CacheStore;

impl ash_core::StoreTag for PrimaryDb {}
impl ash_core::StoreTag for AnalyticsDb {}
impl ash_core::StoreTag for CacheStore {}
```

Built-in convenience aliases can be provided for common single-engine stores:
```rust
pub struct SqliteStore;
pub struct MemoryStore;

impl StoreTag for SqliteStore {}
impl StoreTag for MemoryStore {}
```

---

### 3.2 Resource DSL: The `store <Type>;` Directive

In the `resource!` macro, resources can optionally declare the `StoreTag` they bind to:

```rust
resource! {
    resource User;
    table "users";
    store PrimaryDb; // <--- Type-level store binding

    attributes {
        id: Uuid [pk],
        email: String,
    }
}

resource! {
    resource AuditLog;
    table "audit_logs";
    store AnalyticsDb; // <--- Routed to analytics store

    attributes {
        id: Uuid [pk],
        action: String,
    }
}
```

If `store <Type>;` is omitted, the resource defaults to `DefaultStore`:
```rust
resource! {
    resource Post;
    table "posts";
    // Defaults to `type Store = DefaultStore;`
}
```

#### Codegen: Associated Type on `Resource`

The macro generates an associated type on the `Resource` trait implementation:

```rust
pub trait Resource: Sized + Send + Sync + 'static {
    type Store: StoreTag;
    const DEF: ResourceDef;
    // ...
}

impl Resource for User {
    type Store = PrimaryDb;
    // ...
}
```

---

### 3.3 `StoreRegistry`: Type-Safe Heterogeneous Store Map

`ash-core` provides `StoreRegistry`, using Rust's standard `TypeId` map pattern (similar to Tower / Axum / Actix `Extensions` / `State`):

```rust
use std::any::TypeId;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct StoreRegistry {
    stores: HashMap<TypeId, Arc<dyn DynDataLayer>>,
    default: Option<Arc<dyn DynDataLayer>>,
    schema_supporters: Vec<Arc<dyn DynSchemaSupport>>,
}

impl StoreRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a data layer for a specific `StoreTag`.
    pub fn with_store<T: StoreTag, D: DataLayer + 'static>(mut self, layer: D) -> Self {
        self.stores.insert(TypeId::of::<T>(), Arc::new(layer));
        self
    }

    /// Set the fallback data layer for `DefaultStore` or untagged resources.
    pub fn with_default<D: DataLayer + 'static>(mut self, layer: D) -> Self {
        self.default = Some(Arc::new(layer));
        self
    }

    /// Register schema migration / table creation support.
    pub fn with_schema_support<S: SchemaSupport + 'static>(mut self, supporter: S) -> Self {
        self.schema_supporters.push(Arc::new(supporter));
        self
    }

    /// Retrieve the data layer for a specific resource.
    pub fn get_layer_for<R: Resource>(&self) -> Result<&dyn DynDataLayer> {
        let type_id = TypeId::of::<R::Store>();
        if let Some(layer) = self.stores.get(&type_id) {
            return Ok(&**layer);
        }
        if let Some(layer) = &self.default {
            return Ok(&**layer);
        }
        Err(Error::Invalid(format!(
            "no data layer registered for store `{}` on resource `{}`",
            std::any::type_name::<R::Store>(),
            R::DEF.name
        )))
    }
}
```

---

### 3.4 Compile-Time Store Presence Verification (Optional Strict Mode)

For applications desiring guaranteed compile-time proof that required databases are present in the context, a marker trait constraint can be optionally applied:

```rust
pub trait HasStore<T: StoreTag> {
    fn get_store(&self) -> &dyn DynDataLayer;
}
```

Action invocation methods on builders can optionally constrain `Context<D>`:
```rust
impl<R: Resource> Changeset<R> {
    pub async fn commit_strict<D>(self, ctx: &Context<D>) -> Result<R>
    where
        D: HasStore<R::Store>,
    {
        // Guaranteed at compile-time to have R::Store!
    }
}
```

This prevents runtime misconfigurations before the program ever runs.

---

### 3.5 Third-Party Data Layer Ecosystem (`ash-clickhouse`, `ash-redis`, etc.)

External crates do not need to fork or update `ash-core`. They only need to implement `DataLayer`:

```rust
// In external crate: ash-clickhouse
pub struct ClickHouseDataLayer {
    client: clickhouse::Client,
}

impl ash_core::DataLayer for ClickHouseDataLayer {
    // Implements run_query, create, etc.
}
```

The consumer application connects everything together in `main.rs`:

```rust
let primary_sqlite = ash_sqlite::Sqlite::connect("primary.db").await?;
let clickhouse = ash_clickhouse::ClickHouseDataLayer::connect("http://localhost:8123").await?;

let registry = StoreRegistry::new()
    .with_store::<PrimaryDb>(primary_sqlite)
    .with_store::<AnalyticsDb>(clickhouse);

let ctx = Context::new(registry);
```

---

## 4. Cross-Store Operations & Transactions

When an application uses `ctx.multi()` across multiple stores:
```rust
ctx.multi()
    .create("user", User::create(&ctx).email("bob@example.com"))     // runs on PrimaryDb
    .create_from("log", |ctx, res| {
        let u: &User = res.get("user").unwrap();
        AuditLog::create(ctx).action(format!("Created user {}", u.id)).changeset() // runs on AnalyticsDb
    })
    .commit()
    .await?;
```

The multi pipeline coordinates each step through `StoreRegistry`:
- Operations targeting `PrimaryDb` are dispatched to `primary_sqlite`.
- Operations targeting `AnalyticsDb` are dispatched to `clickhouse`.
- If a step fails, compensation or store-level rollback is invoked where supported by the underlying engine.

---

## 5. Backward Compatibility & Migration Path

1. **Existing single-store usage** (`Context::new(Memory)` or `Context::new(Sqlite)`) remains **100% compatible**. Single data layers automatically act as the `DefaultStore`.
2. **Existing `data_layer sqlite;` / `data_layer memory;` directives** will remain supported as aliases to `store SqliteStore;` and `store MemoryStore;`, issuing a soft deprecation note in documentation.
3. Resources without any `store` declaration continue to work transparently via `DefaultStore`.

---

## 6. Implementation Checklist

- [x] **Phase 1: Core Types**: Add `StoreTag`, `DefaultStore`, `SqliteStore`, `MemoryStore` in `ash-core::store`.
- [x] **Phase 2: Registry Evolution**: Update `StoreRegistry` (formerly `DataLayerRegistry`) to support `TypeId` indexing via `.with_store::<T, D>(layer)`.
- [x] **Phase 3: Macro Updates**:
  - Add `store <Ident>;` syntax support to `ash-macros::define::parse`.
  - Emit `type Store = <Ident>;` in `ash-macros::define::codegen::resource`.
- [x] **Phase 4: Engine Integration**:
  - Update `ash-core::engine` (`get`, `create`, `update`, `destroy`, `query`, `multi`) to route via `R::Store` and `store_type_id`.
- [x] **Phase 5: Test Coverage**:
  - Multi-instance SQLite routing test (`PrimaryDb` vs `AuditDb`).
  - Mixed SQLite + Memory store routing test.
  - Mock external 3rd-party data layer test.
  - Missing-store diagnostic error test.
