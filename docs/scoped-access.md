---
read_when:
  - Working on authentication tenant scoping or RPC request context
  - Migrating direct Context.data reads into the core engine
  - Changing record presentation, field policies, hooks or response adapters
  - Implementing a data layer or changing upsert conflict behavior
  - Registering event readers, notifier grants or shared subscription groups
---

# Scoped access

`Context::access()` returns a borrowed `ScopedAccess` handle. Its reads use the
context's actor and tenant. `ReadRequest` supplies query shape and trusted read operations, so it cannot replace
execution identity.

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
private authorization mode. The policy planners and executable reads use the sealed
identity from the read-only `authorization_actor()` bridge. Ordinary binding strips
that compiler marker; actor IDs, roles and attributes cannot create system authority. Keep Runtime
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
`ReadRequest` accepts trusted Rust `Filter` and `Sort` values. Client parsers return
`ClientExpr`; its filter marker survives composition until the core planner applies
field visibility and resolves relationship scopes. Generated preparations return
`TrustedExpr`. See [Client read plans](client-read-plans.md) for parser APIs, Runtime
cursor-key configuration and supported subscription predicates.

## Guarded upsert conflicts

Single creates with `upsert_on` and bulk creates with `BulkCreateOptions::upsert`
use the same core-owned `UpsertPlan`. Core builds it after action preparation from
the bound scope, action, identity and supplied update columns. Its private fields
bind the resource, insert values, tenant namespace, conflict predicate and exact
changed values. Data layers receive a borrowed plan with read-only accessors.

An `all_tenants` identity keeps uniqueness global. It does not grant permission to
update or return a row in another tenant. The conflict branch checks attribute
tenancy, the create action's write policy against the stored row, and expected
optimistic-lock versions when that action declares them. The prospective insert
must also pass normal create authorization. A rejected conflict returns
`Error::NotFound`, displayed as `not found`, for tenant, policy and version failures
alike. It contains no record, identity values, tenant or conflicting primary key.
Bulk operations use the same error per failed row under their existing transaction
and stop-on-error options. This outcome does not hide the existence of a global
uniqueness collision; it prevents that collision from exposing the row's contents.

System scopes bypass the action policy but retain tenant and version guards. A
system context bound to tenant A cannot update or receive tenant B's row through an
`all_tenants` conflict. Bind a separate context to B for that work. Resources that
explicitly declare `global: true` retain their documented tenantless behavior.

An empty update list changes no columns. It returns the existing authorized row,
with its original ID and tenant. Nonempty lists change only the named values.
Unknown fields, absent update values, primary-key columns and tenant columns are
rejected before the upsert runs. On a locked create action, nonempty conflicts also
advance each version after checking its supplied value. Empty conflicts check the
versions without advancing them. The initial insert retains the prepared versions.

Postgres and SQLite compile the predicate inside
`ON CONFLICT (...) DO UPDATE SET ... WHERE ... RETURNING *`. A no-change conflict
assigns the stored primary key to itself, rather than copying an incoming identity
value. Both dialects return only the affected row. SQLite uses `RETURNING` for
upserts and has no identity reselect fallback; an empty result ends immediately as
`NotFound`. Partial identity predicates remain part of the conflict target, and
null handling remains the backend's existing identity contract.

Memory finds the conflict, checks the predicate, validates resulting identities and
writes the selected values under one store write lock. Empty updates preserve the
stored record and its table key. Memory still rejects partial-identity upserts
because their predicates are SQL.

Data layers and SQL upsert helpers accept the opaque plan. The former
`upsert_raw` and `compile_upsert_raw` entry points have been removed.

## Implementing a data layer

This is a breaking change to the DataLayer extension interface. Execution methods
accept borrowed, immutable plans built only by core:

- `create`, `update`, `destroy`: `CreatePlan`, `UpdatePlan`, `DestroyPlan`.
- `run_query`, `count`, `run_query_with_count`: `QueryPlan`, `CountPlan`,
  `QueryWithCountPlan`.
- `run_query_per_key`: `PerKeyPlan`, including scoped destination and join predicates.
- `update_atomic`, `destroy_atomic`: `AtomicUpdatePlan`, `AtomicDestroyPlan`.
- `bulk_create`, `bulk_update`, `bulk_destroy`: the corresponding `Bulk*Plan`.
- `upsert`: `UpsertPlan`, now also retaining the complete bound scope.

Every plan exposes `scope()` and `resource()`. Scope provides read-only `actor()`
and `tenant()` accessors. Write plans expose their prepared key and fields. Read
and atomic plans expose the compiled query, including policy predicates, the
policy compiler's authorization identity and the resolved tenant namespace.
For attribute tenancy, the namespace can be absent while the scope still retains
the tenant and the query contains its tenant predicate. Never replace either.
`CompiledQuery` is still public for compile-only tooling; it grants no execution.

```rust,ignore
async fn run_query(&self, plan: &ash_core::QueryPlan<'_>) -> ash_core::Result<Vec<ash_core::FieldMap>> {
    let sql = compiler.compile_select(plan.resource(), plan.query())?;
    // Execute on this adapter's private connection and return rows to core.
    self.fetch_rows(sql).await
}
```

Custom adapters must honor every planned predicate, conflict guard, tenant and
atomic condition. They remain trusted implementations. Rust prevents callers from
constructing execution plans; it cannot prove an adapter's SQL correct. Default
batch implementations forward the immutable single-row child plans. Override a
batch only when its optimized statement preserves those same decisions.
`DynDataLayer` uses the same plans, and `StoreRegistry` forwards them intact,
including the combined page/count operation. Capability checks, schema installation
and transaction callbacks retain their existing signatures.

Migrate implementations by replacing their raw argument lists with the matching
plan and reading its accessors. Remove `upsert_raw` implementations. SQL compiler
methods other than the removed raw upsert helper remain compile-only building
blocks for adapters and `cargo-ash`; they cannot execute against a Context.

Application code must replace `ctx.data` and `ash_core::insert` with resource
actions or `ctx.access().read`. Bind a system scope at startup for seeds and jobs;
keep tenant binding explicit. `ReadRequest` also supports selections, calculations,
arguments and aggregates. Counts and counted reads use `access().count` and
`access().read_with_count`. Core runs the named read's preparations, scopes the
query, runs after-read hooks and returns Views. Cursor sort values are sealed
before hooks and field projection when the request supplies a keyset ordering.
For pagination, `prepare_read` returns an opaque `PreparedRead` retaining the exact
scope, action arguments and mandatory candidate predicate. Its `query()` is read-only.
`read_prepared`, `count_prepared` and `read_prepared_with_count` accept identity-free
page shapes, intersect their filters with that predicate and reject another bound
scope. They run preparations once, including for combined page/count reads. A
transaction retaining the same scope can execute the prepared read.

Authentication receives separate bootstrap-issued, resource-bound read and write
grants. `Runtime::credential_writes::<R>(fields)` names the only credential
attributes it may mutate. `update_credentials` rejects primary-key and tenant
changes, checks attribute types and retains the bound tenant. These grants belong
in trusted authentication configuration, never request input. Durable session
consumption and credential versioning remain separate work.

Credential grants are bound to the Runtime that issued them. Use that Runtime, its clones or
explicitly derived `with_backend` runtimes to bind authentication contexts; Runtime
transactions retain the same authority. A grant from an unrelated Runtime returns `Forbidden`, even if both runtimes
wrap the same backend. Constructing an unrelated Runtime cannot grant credential
access to an existing Context.

Schema installation also requires startup authority. Use
`runtime.install(resources)` or the generated associated function
`MyDomain::install(&runtime)`. `Context::install` and the old zero-argument domain
installer are removed. Resource statements may contain arbitrary migration SQL;
a request Context cannot execute them. Installation and migration APIs on a backend
retained by startup code remain available.

## Outside the guarantees

Keep an application-owned database pool or connection for reporting and migrations.
For example, clone the pool before moving a backend into `Runtime`. SQL compiled
and executed through that pool does not run framework tenancy, policies, action
changes, hooks or View projection. The application owns those decisions. A Context
has no backend or connection accessor, and cannot recover Runtime authority.
Backend installation and migration tools remain startup responsibilities.

Trusted server code holding Runtime can deliberately create system scopes or bind
another actor and tenant. Trusted SQL fragments can contain developer-written
subqueries. Resource metadata and adapter implementations are also trusted code.
These capabilities remain explicit; the plan interface protects code handed only a
bound Context from omitting the core planner during framework execution.

## Managed relationships and cascades

A supplied belongs-to or many-to-many destination ID resolves through the
resource's default read action before core copies any key into a foreign key or
join row. Resolution applies tenant predicates, preparations, after-read hooks and
row policies. Composite belongs-to relationships copy every referenced column
from that resolved row. Unknown IDs, foreign-tenant IDs and IDs hidden by read
policies or preparation filters return `Error::NotFound`, with no record details.
Malformed IDs have the same outcome. A belongs-to input accepts one record.

System scopes bypass row policies but still enforce tenancy and preparations.
They cannot link a destination in another tenant. Bind a separate scope inside
`runtime.transaction` for intentional work across tenants.

Managed creates and updates run the destination's actions. Join creates and
destroys run the join resource's actions, including changes, validations, policies,
tenant stamping and hooks. Missing actions fail instead of falling back to raw
writes. Link columns are forced by core after checking the destination; caller
input cannot replace them. Existing has-many DirectControl behavior remains:
only a current child can be updated by its supplied ID. An unmatched ID creates
a new child with a new key and leaves the named row alone.

Core retains stored link keys internally until linking finishes, even when field
policies hide those columns from the returned View. Discovery of existing children
and joins uses the same scoped read planner as destination resolution.

Before discovering cascade children, core re-resolves the stored parent and checks
its destroy policy. Caller-supplied parent fields cannot select another set of
children. Discovery includes tenant and read-policy predicates, and child writes
retain their own action authorization and tenant guards. Hard cascades include
archived children by omitting only the exact soft-delete visibility filter declared
by `ResourceExtension::soft_delete_attribute`. Other preparations and read hooks
still run. An archival child without a hard destroy action retains referential hard
delete behavior, checked against its declared destroy policy. Join cascades require
a hard destroy action. Nilify uses an update action, or an implicit scoped update
when the resource has none.

Managed actions and cascades require a storage transaction, including when the
caller disabled bulk batch transactions or attached a notification buffer. A later
failure rolls back the parent and all earlier related writes. Bulk managed rows
prepare inside that transaction so creating a belongs-to record cannot escape it.
Soft cascades reached by an atomic update also open a transaction. Notifications
from failed work are discarded; successful nested work joins the outer buffer.

SQLite and Postgres use their real transactions and nested savepoints. Memory uses
a private snapshot for each transaction, with nested snapshots acting as savepoints.
Commit compares the entire live store with its starting snapshot under the store's
write lock, then publishes all changes together. Any concurrent store change returns
`Error::DataLayer` with a transaction-conflict message and publishes none of the
snapshot. Applications may retry the whole operation. This deliberately rejects
unrelated concurrent changes too, preventing phantom discovery results and unique
identity races. Memory now supports transactional bulk batches as well.

A backend whose `DataLayer::in_transaction` supplies `None` returns
`Error::AtomicityUnsupported` before a multi-record action writes. A notification
buffer alone grants no transaction capability. StoreRegistry has no coordinated
transaction across its stores, so it rejects managed actions, cascades and explicit
Context/Runtime transactions. `BulkTransaction::All` also rejects unsupported
backends. Ordinary bulk batches keep their existing capability check; `Off` runs
independent rows, while each managed row or cascade still requires its own transaction.

Breaking behavior changes include missing managed actions, unresolved supplied IDs,
unsupported explicit transactions, and memory transaction conflicts on any concurrent
store change. `Error::AtomicityUnsupported` is a new public error variant.
`ResourceExtension::soft_delete_attribute` is an additive method with a default of
`None`. No dependencies are added.

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

Step 5 applies dependency visibility to calculations and projects aggregate source
values for the reader. See [Derived values](#derived-values). Private committed events use the same presenter for each subscriber or explicitly
bound group scope. See [Events and subscriptions](#events-and-subscriptions).

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
the schema contains a privileged default. Both protocols validate connection-init
parameters through `RequestData::connection_init` before acknowledging the connection.
Legacy subscriptions keep unshared streams, while honoring explicit group identity.

## Events and subscriptions

Breaking changes replace raw notification maps and string-only subscription groups.
A `CommittedEvent` is opaque and core-created. It retains the stored snapshot and
preimage for admission, including after destroy. It exposes no raw field or actor
accessors, argument bag, constructor or serialization. Debug output is redacted.
Transaction queues retain these opaque events and dispatch only after commit.

`Notification` now contains `record: View`, `previous: Option<View>`, resource/action
identity, record ID and tenant. Use `notification.get(name)`, `record.fields()` or
`previous(name)` for projected values. `record_fields`, `previous_fields`, `actor`,
`Notification::new` and `with_tenant` are removed. Default notifiers read as anonymous
in the event's tenant. A record that the default read excludes produces no notification.
Arguments are never copied, and arbitrary context/changeset metadata is no longer
exported. Metadata starts empty. Actor attributes are never exported.

Register a reader from a bound context and use it for PubSub or a `ScopedNotifier`:

```rust,ignore
let reader = ctx.event_reader(&Ticket::DEF, client_filter)?
    .select(vec!["id".into(), "title".into()])
    .with_metadata(&[MetadataDef::new("request_id", AttrType::String)]);
let mut events = pubsub.subscribe_reader(reader, "ticket:*")?;
let event = events.recv().await?;
assert_eq!(event.tenant.as_deref(), ctx.tenant());
```

Metadata declarations name both the key and its type. Core recursively projects
resource-valued metadata. The previous snapshot must independently pass the reader's
read predicate before its fields are projected. An ownership transfer therefore does
not disclose a former owner's preimage to the new owner.

All PubSub registration requires a resource and bound context, or an `EventReader`:
`pubsub.subscribe(&ctx, &Ticket::DEF, "ticket:*")?`. Typed helpers also take `&ctx`
and return `Result<Subscription>`: `Ticket::subscribe_all(&pubsub, &ctx)?`,
`Ticket::subscribe_action(&pubsub, &ctx, "create")?` and
`ticket.subscribe(&pubsub, &ctx, None)?`. `TopicKey` separates `tenant` from the
pattern. Wildcards and delimiter characters in tenant names cannot cross that
boundary. Core also checks the event's exact tenant and issuing Runtime independently
of topics, then rechecks the default read and evaluates the client filter against the
private snapshot. Queues contain only admitted, projected notifications. The old
shared raw `Arc<Notification>` fan-out is gone. Dropping a subscription removes its
queue. Publish methods now accept an opaque `&CommittedEvent` and return `Result<usize>`,
counting admitted subscriber queues rather than matched patterns.
Custom topic formatters receive the reader's projected notification.

Unsupported relationship, calculation, aggregate, trusted SQL and page-boundary
predicates fail at registration, including dependencies in default-read preparations.
A required missing tenant also fails registration. `NeedsData` never admits an event.
Selecting relationships and derived fields in GraphQL remains supported through
scoped follow-up reads. Snapshot filtering does not execute those reads.

Mail and logging consumers needing additional fields declare a Runtime grant:

```rust,ignore
let mail = runtime.notifier_grant::<Order>(&["customer_email", "amount"])?;
let runtime = runtime.with_notifier(mail.attach(Arc::new(email_notifier)));
let log = runtime.notifier_grant::<Order>(&[])?
    .with_actor_id()
    .with_metadata(&[MetadataDef::new("request_id", AttrType::String)]);
```

A grant is resource-bound and valid only on its issuing Runtime and clones. It can
receive events across that Runtime's tenants, with only its named stored attributes,
and deliberately bypasses row and field policies for those fields. Actor IDs and
metadata require separate declarations. It grants no action arguments or actor
attributes. Treat attaching such a sink as trusted startup configuration. Reissue
grants when constructing an unrelated Runtime, even over the same backend.
`runtime.with_backend(registry)` deliberately derives a Runtime with the same
application authority, notifiers and grants over another startup-configured backend.
Use it when a StoreRegistry and direct store writes publish to the same broker.
It is unavailable on Context; backend identity still separates shared stream keys. Authentication
reset tokens continue through `AuthSender`/`AuthMailerSender`, not broadcasts.
Authentication extensions mark configured password and API-key hashes sensitive.
Ordinary Views hide them, and GraphQL/TypeScript generated schemas omit them. Existing
applications must use credential permits for credential reads; system contexts and
explicit field grants retain deliberate access.

`RequestData::apply` now returns `BoundRequest`, replacing `(Request, String)`:

```rust,ignore
fn apply(&self, headers: &HeaderMap, request: Request) -> BoundRequest {
    let member = self.runtime.bind(authenticate(headers), verified_tenant(headers));
    BoundRequest::for_subscription_group(request, member, self.public_staff_context.clone())
}
```

Use `BoundRequest::new` to resolve each authenticated subscriber in its own
scope. For grouping, bind and retain the generic actor's context at startup, then
clone it here. Subscription execution replaces the member context with that group
context before resolving anything. HTTP, WebSocket queries and mutations retain the
member context. The key contains the complete sealed scope and backend identity,
plus document, operation name and variables. Authenticated bindings share only when
cloning the same bound group; matching IDs, roles or app strings cannot merge them.
Anonymous ordinary bindings share by default within one Runtime, backend and tenant.
System authority, another Runtime or another tenant never joins that public group.
Request metadata is not a grouping key; applications must put every authorization
input in the bound actor or tenant and use an explicit group for group-specific
execution. The generic context supplies the group's metadata too.

`RequestData::connection_init(headers, params)` validates supported credentials and
returns the headers used for subsequent binding. Implement verification and define
header/parameter precedence in this server method. The default accepts absent/null
parameters and an empty object, and rejects nonempty or malformed parameters. A
rejection closes either protocol with code 4403 before acknowledgment; supplied
credentials are never silently ignored. The framework does not interpret arbitrary
connection parameters as trusted actor or tenant data.

## Limits

Steps 2 through 8 bind execution scope, normalize client reads, present results,
protect derived values and require core-built plans for backend execution.
`Context::data`, the raw insert function and raw upsert compatibility methods have
been removed. Backend transport uses maps only between adapters and core. Hooks
use RawRecord; ordinary extension reads return View. Credential issuance is
restricted to Runtime, while credential lookup results remain privileged maps.

Cross-tenant lookup grants still need a resource/action declaration and a backend
contract for discovering tenant namespaces. Lagrange therefore still requires a line
at login. Subscription groups bind their own complete scope and resolve as the group
actor. Keyset cursors are sealed by Runtime in step 3. Backend-specific trusted
plan expressions are implemented. See [Trusted data-layer nodes](trusted-data-layer-nodes.md). Scope has no public constructor or layout,
so those additions do not require changing the binding API.

This change does not fix token namespace isolation, durable access revocation,
deleted-user validation, session invalidation or atomic reset/refresh consumption.
Live credential revocation and subscription termination on revocation remain follow-up work. The design and the full migration sequence are in
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

## Derived values

Calculations inherit the visibility of every operand, including conditions and
unused coalesce branches. If any operand is unreadable on that record, the
calculation is forbidden and presents as null. This also applies when the stored
operand is null. Explicit policies on a calculation add restrictions to that rule.
Hidden calculations are skipped before evaluation, so callback errors and arithmetic
overflow cannot expose unreadable inputs. Explicitly hidden aggregates are skipped
too. Client filters and sorts compare the projected value, including null checks and
negation. Trusted preparations can still compare or order by the stored operands.
Sealed keyset cursors retain that effective ordering without exposing its values.

A calculation may explicitly declare `declassify` to expose a reviewed derived
value over hidden inputs. The declaration waives inherited visibility only;
policies on the calculation itself still apply. Review both the expression and its
arguments before granting this permission. A constant, concatenation or coalesce
is not an implicit declassification.

```rust,ignore
calculations {
    salary_x2: Option<i64> = coalesce(salary, 0) * 2;
    ssn_last_4: Option<String> [depends_on [ssn], declassify] = custom(last_four);
}
field_policies {
    field ssn { authorize_if actor_attribute_equals(role, "admin"); }
    field salary_x2 { authorize_if actor_present; }
}
```

Every opaque Rust calculation must declare `depends_on [fields]`, even if the list
is empty. Its callback receives only those fields plus the expression's statically
known dependencies. The framework rejects opaque calculations without a dependency
declaration. A callback cannot inspect other record fields through its input map. The declaration also
contributes to visibility for non-declassifying calculations. A callback depending
on a whole embedded value requires its nested fields to be readable too. Any nested
forbidden field hides that calculation, including when the callback would only use
another part of the object. Declassification explicitly grants the declared raw
composite input. Handwritten metadata
uses `CalculationDef::dependencies(&["ssn"]).declassify()`. Custom callbacks remain
Rust-only; SQL filters and sorts cannot execute them. Handwritten calculation
metadata follows the DSL's existing stored-field dependency rule too. Derived-field
and unknown-field dependencies fail explicitly, including on an empty table.

Aggregates authorize destination and join rows as before. The value-reading kinds
project their source field under each source record's policies inside the query:

- `sum` ignores hidden and null operands and returns null if none are visible.
- `first` chooses the first authorized row by ascending primary key, then projects
  its operand. A hidden or null value produces null; it never skips that row to
  find a visible value. Resources without a primary key retain backend row order.
- `count` and `exists` depend on authorized rows, independently of field visibility.

These are the four supported aggregate kinds. Policies on the aggregate itself
can further restrict its result. Client aggregate comparisons and sorts use the
same projected operands. Trusted preparations retain their original stored-value
semantics. Data layers compute a raw derived value only when a trusted filter or
sort needs it, and carry its projected counterpart privately until presentation or
cursor sealing. Hidden inputs are never summed or evaluated merely to discard the
result. Internal query column names beginning
with `__ash_` are reserved and cannot be declared as resource fields.

Reads, write-result loads, GraphQL and RPC all use these rules. GraphQL reports a
forbidden-field error for inherited or explicit calculation restrictions and explicit
aggregate restrictions. An aggregate whose source values project to null has a null
value without an extra forbidden-field error. System scopes bypass field and row
policies, while still requiring valid custom calculation declarations.

## Trusted data-layer operations

Scoped reads accept developer-owned SQL fragments, Postgres row locks and
`distinct_on` through [trusted data-layer nodes](trusted-data-layer-nodes.md).
The planner retains tenancy and row policies; `View` retains field policies and
calculation dependency visibility. Unsupported backends return a typed planning
error before reading data. Read locks require a real transaction. These operations
remain available through executable plans after raw execution is removed.
