---
read_when:
  - Working on authentication tenant scoping or RPC request context
  - Migrating direct Context.data reads into the core engine
  - Changing record presentation, field policies, hooks or response adapters
---

# Scoped access

`Context::access()` returns a borrowed `ScopedAccess` handle. Its reads use the
context's actor and tenant. `ReadRequest` supplies only filter, sort, limit and
offset, so it cannot replace execution identity.

## Bind at the application boundary

`Runtime<D>` owns the backend and bootstrap authority. Authenticate the actor and
verify tenant membership in trusted application code, then bind one request scope:

```rust,ignore
let runtime = Runtime::new(data).with_notifier(notifier);
let ctx = runtime.bind(resolved_actor, resolved_tenant);
let strategy = User::auth_strategy(&runtime);
```

`bind` accepts `Option<Actor>` and `Option<String>`. Actor and tenant are private,
immutable members of the context's shared scope. `actor()` and `tenant()` borrow
them. A request that holds only a context cannot create or replace its identity.
On an ordinary Runtime, `bootstrap()` is shorthand for the anonymous, tenantless
`bind(None, None)`.

Seeds, migrations and background jobs can deliberately bypass authorization:

```rust,ignore
let system = runtime.system();
for tenant in tenants {
    let ctx = system.bind(None, Some(tenant));
    Ticket::create(&ctx).title("Seeded").await?;
}
let globals = runtime.system().bind(None, None);
```

`system()` returns privileged Runtime authority. Its bound contexts bypass row and
field policies. They still run action changes, validations and hooks, and enforce
tenancy, including required tenants. System authority is separate from the actor:
`bind(None, tenant)` retains an absent actor, and binding a supplied actor preserves
its identity for changes, validations, hooks and notifications. Scope stores the
private authorization mode. The existing policy compilers receive a sealed identity
through the read-only `authorization_actor()` bridge until executable plans replace
that interface. Ordinary binding strips that compiler marker; actor IDs, roles and
attributes cannot create system authority. Keep Runtime
and system contexts in trusted application code. Hooks receive no system capability
unless application wiring explicitly supplies one. Declared hook capabilities are
follow-up work.

Context clones, metadata changes, notification buffers and `Context::transaction`
retain the exact same scope. A changeset records its preparation scope and rejects a
commit through any other scope before running hooks or writes. Even another `bind`
with identical actor and tenant creates a different scope. Queries, changesets and
generated action builders have no tenant setters.

`Runtime::transaction` supports workflows that deliberately need several scopes on
one transaction backend. Bind each operation inside its closure. All contexts it
creates use the transaction backend and share its notification buffer. An error
rolls back their writes and drops their queued notifications. A context never changes
tenant in a transaction. Changesets prepared outside the closure must retain their
original scope and cannot be transferred to a new binding inside it.

## Ordinary reads

```rust,ignore
let rows = ctx.access().read(
    &Ticket::DEF,
    "read",
    &FieldMap::new(),
    ReadRequest { limit: Some(10), ..Default::default() },
).await?;
```

`read` resolves the resource's named read action and applies its preparations,
argument filters, row policies, tenant scope and after-read hooks. An unknown action
or an action of another kind fails before querying. Attribute tenancy adds a
predicate; context tenancy selects the backend's tenant namespace. A non-global
tenant resource requires a tenant. Explicit resource-level `global: true` retains
its existing tenantless behavior.

These reads return `View`s with reader-specific projected values and forbidden-field
masks. `view.fields()` and `view.into_fields()` expose projected values only.
`ReadRequest` accepts trusted `Filter` and `Sort` values. Adapters still
must guard client expressions with the existing input policy helpers.

## Reader-specific results

Core presents ordinary execution results before returning them. The private presenter
computes field-policy masks from each intact record, then selects and projects its
fields. It recursively handles embedded resources, arrays, typed maps, union members
and loaded relationships. Policy operands and load keys remain available until this
step; hiding an operand cannot change another field's policy decision. System scopes
retain all fields through the same presenter.

Dynamic writes and scoped reads return `View`. Typed query and action builders keep
returning their resource structs, constructed only from a completed View. Those
projected structs can still derive `Serialize`. Context-free `build_*().build()` and
`Resource::from_fields` construct application data without a reader; they are trusted
input construction APIs, not execution responses.

Write `after_action` hooks receive non-serializable `RawRecord<R>` for typed builders
or `RawRecord` for dynamic hooks. Read preparations receive `&mut [RawRecord]`.
Use `get`, `insert`, `fields` and `fields_mut` to inspect or change engine data.
`Changeset::data` and `into_record` also expose RawRecord. Dynamic
`after_transaction` hooks receive `View`; typed hooks receive projected resource
structs. Hooks must finish before presentation so their changes cannot escape field
policies.

Manual persistence callbacks accept and return `RawRecord`, then core runs the
normal finish stage, including hooks, notifications and presentation:

```rust,ignore
Employee::intake(&ctx).name("new employee").persist(|_ctx, mut raw| async move {
    raw.insert("salary".into(), Value::Int(42));
    external_store.save(raw.fields()).await?;
    Ok(raw)
}).await?;
```

Generic action runners retain the return type's `AshType` presentation contract.
Resource results, arrays and resource-bearing `AshTypedMap`/`AshUnion` values use
recursive presentation too. Custom Rust results without Ash metadata retain their
application-defined serialization; use an Ash type for resource-bearing results.
The low-level `run` callback now returns `returned::ActionOutput<T>`. Core returns
`returned::ActionResult<T>`, retaining the projected tree and typed value together.
Generated builders return the typed value; `Resource::run_generic` returns the
`PresentedValue` tree so adapters retain the original masks. Generated return-type
resolvers and `ActionDef::result_type()` preserve resource/composite metadata for
custom runners and schema generation. These helpers avoid rechecking policies on a
record whose policy operands have already been hidden.

GraphQL resolvers consume Views and use their recorded forbidden masks. Relationship
keys have separate private storage and never enter the serialized fields.
`view.relationship_source(rel)` returns a non-serializable `RelationshipSource`;
core relationship loaders and counters accept these handles. Adapters cannot read
or serialize their key values. Trusted callers with their own source maps can
explicitly construct a handle with `RelationshipSource::from(fields)`. Preload
caches accept only presented child Views. RPC rendering accepts Views and emits only
the request's selected fields, including nested selections. Neither adapter computes
field-policy masks from an already redacted record.

Derived-value visibility is step 5. Calculations and aggregates still have their
existing visibility semantics. This step does not declassify or suppress their
inputs. Step 9 will feed private committed events through the same presenter for
each subscriber or bound group scope. Notification and PubSub payloads remain
unchanged; the existing GraphQL subscription response path presents each event for
its current reader.

## Credential reads

Authentication needs password and API-key hashes before a user can pass ordinary
read policies. Trusted server configuration calls
`runtime.credential_reads::<User>()`. The permit grants credential reads of that
resource only:

```rust,ignore
let rows = ctx.access().credentials(
    &permit,
    Filter::eq("email", email),
    2,
).await?;
```

Credential reads bypass read policies, preparations, hooks and field redaction.
They always apply the context's tenant scope, including the required-tenant check.
The result is raw credential data and must stay inside authentication code.

The permit constructor is private to core. `AuthStrategy::new(&runtime)` and the
generated `User::auth_strategy(&runtime)` obtain a permit for that resource from
Runtime. Neither a request parser nor a bound context can issue a permit. Credential
results remain raw maps until the later credential-result migration.

All four `AuthStrategy` user lookups now use credential reads: subject, password
identity, reset-request identity and API-key hash. The adapter must select the tenant
before calling any of them. Tenantless login to a non-global tenant resource now
fails instead of searching across tenants.

`require_tenant_match` compares a signed tenant to the context exactly, including
`None`. Refresh and password-reset operations check it before consuming a token or
writing a password. The standard auth router also checks access-token tenancy before
`/me` and `/change-password`. Its supplied context selects the tenant for every route.
`AuthUser` preserves the signed tenant in the actor's `tenant` attribute. Custom
adapters must still bind or compare it with their selected tenant.

Lagrange demonstrates adapter migration: `/auth/sign-in` requires a `line` alongside
email and password. Its app-level password and API-key methods also take a line and
bind it before looking up credentials. Knowing a line is insufficient to sign in;
the credentials must identify a user or transponder within that line.

## RPC compatibility

`Rpc::run` and `Rpc::validate` ignore payload `tenant` values. The HTTP and Phoenix
channel adapters select the tenant before parsing requests. A payload cannot replace
that tenant or supply a missing one. RPC record lookup uses ordinary scoped reads,
including the read action's hooks.

This changes payload-tenant precedence. Apps that previously supplied only a payload
tenant must move tenant resolution and membership checks into their server adapter.
The generated TypeScript client retains its optional `tenant` property for
AshTypescript wire compatibility; ash-rust ignores it. Use the adapter's verified
headers or channel connection credentials to select tenancy. GraphQL uses only the bound context its server adapter supplies. A separate
`Actor` in GraphQL request data cannot fill or replace that identity.

`graphql_router(schema, context)` and
`graphql_router_with_hub(schema, hub, context)` require an explicit fixed context.
For per-request identity use `graphql_router_with` and implement `RequestData` by
calling Runtime's binder. HTTP, `graphql-transport-ws` and legacy `graphql-ws` all
apply this binder. A network request without a request-local context fails even if
the schema contains a privileged default. Connection-init credential processing
remains follow-up work. Legacy subscriptions keep their existing unshared behavior.

## Limits

Steps 2 and 4 bind execution scope and centralize result presentation.
`Context::data` and raw `insert` remain
public but deprecated until engine reads and writes use executable plans. Core uses a
private backend accessor for those paths. Existing adapter code and raw-backend tests
carry documented deprecation allowances. New caller code still receives warnings.
There is no opaque executable plan yet. DataLayer transports still use raw maps;
core wraps hook and presentation records in RawRecord. Credential issuance
is restricted to Runtime, while credential results remain raw maps.

Cross-tenant lookup grants still need a resource/action declaration and a backend
contract for discovering tenant namespaces. Lagrange therefore still requires a line
at login. Subscription deduplication still trusts the application's string key and resolves
with the first member's context. That stays the case until groups bind
their own complete Scope and resolve as the group actor. Sealed cursors and backend-specific trusted
plan expressions belong to later steps. Scope has no public constructor or layout,
so those additions do not require changing the binding API.

This change does not fix token namespace isolation, durable access revocation,
deleted-user validation, session invalidation or atomic reset/refresh consumption.
It also does not fix managed relationships, upsert conflicts, derived-value
visibility, or subscription authorization and payloads. The design and the
full migration sequence are in
[RFC 0005: Scoped execution](wip/0005-scoped-execution.md).

## Verification

Use Rust 1.90 through mise. Set `DATABASE_URL` to a local test database for the
Postgres suites, for example
`postgres://postgres:postgres@localhost/ashrust_audit_auth`.

```sh
mise exec -- cargo test --locked --all
mise exec -- cargo test --locked -p ash-core -p ash-authentication -p ash-typescript -p ash-graphql --all-features
mise exec -- cargo clippy --locked --all-targets --all-features -- -D warnings
```

Authentication regressions are in `tenant_binding.rs` and
`tenant_binding_postgres.rs`; RPC regressions are in `rpc_scope.rs`. Core scope
regressions are in `request_scope.rs`; HTTP and both WebSocket protocols are covered
by `transport_scope.rs`. The inconsistent
reset-claim fixtures sign and store their tokens directly because tenant-scoped
reset requests can no longer issue those credentials.
