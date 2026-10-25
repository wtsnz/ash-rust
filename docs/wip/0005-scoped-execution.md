# RFC 0005: Scoped execution, and where ash-rust departs from Elixir Ash

- **Status**: Proposed. Steps 1 through 7 implemented, pending review; the rest planned. See §5.
- **Date**: 2026-10-24
- **Authors**: Will
- **Target crates**: `ash-core`, every data layer, `ash-graphql`, `ash-typescript`, `ash-authentication`, `ash-pubsub`

## 1. Summary

ash-rust started as a port of Elixir's Ash Framework. This RFC records the point where it deliberately stops being a direct port.

In Elixir Ash, the actor, tenant and authorization are options passed on each call (`actor:`, `tenant:`, `authorize?: false`), and the protections (tenant filtering, policies, field policies, the input/trusted filter distinction) are applied by the code paths that choose to use them. Ash's design is sound, and Elixir has no way to stop a caller from taking another path.

ash-rust can. It replaces per-call options with **capabilities**: values that grant a power, created in one place and passed explicitly. The core engine becomes the only route to the data layer and the only source of responses. Every power Elixir Ash offers is still available, but each is a named, visible call instead of an option any code can pass.

An audit of ash-rust found its security bugs clustered in exactly the places where code bypassed the engine, set its own tenant, or returned unredacted records. That's the motivation.

## 2. Precursors

None of this is new. It applies well-established ideas to a resource framework:

- **Object capabilities.** Authority comes from holding a reference, not from ambient state. Dennis and Van Horn, *Programming Semantics for Multiprogrammed Computations* (1966); Mark S. Miller, *Robust Composition* (PhD thesis, 2006); the E language. `Runtime`, `Context` and the grants below are capabilities in this sense.
- **Removing ambient authority in Rust.** The Bytecode Alliance's `cap-std` replaces "open any path" with handles to directories you were given. WASI and FreeBSD's Capsicum do the same at the OS level. Here, "query any table in any tenant" is replaced with a handle to one scope.
- **Phoenix 1.8 scopes and `Ash.Scope`.** Phoenix generators thread a `%Scope{}` (actor, tenant) through context functions, and `Ash.Scope` groups actor, tenant and context. This is the direct Elixir ancestor. ash-rust's `Scope` goes further: it's sealed, bound once, and can't be replaced by the code it's passed to.
- **Row-level security bound per connection.** Postgres RLS with a session setting for the tenant (as PostgREST and Supabase use with JWT claims) binds the tenant once and enforces it under every query. The scoped executor does the same in the framework, so it works on every data layer.
- **"Parse, don't validate"** (Alexis King, 2019) and **taint tracking** (Perl's taint mode). Client input gets its own type that can't be used where a trusted expression is expected, instead of a flag a caller must remember to set. Ash's `filter_input` vs `filter` is the same distinction, made by convention.
- **Secret and view types.** The `secrecy` crate's `Secret<T>` keeps sensitive values out of `Debug` and serialization. Serializer and presenter layers separate stored models from responses. `View<R>` combines both: the only type that reaches a serializer is the per-reader, redacted one.
- **Opaque cursors.** The Relay connection spec requires cursors to be opaque to clients. Sealing them (encrypted and authenticated) makes that a guarantee instead of a convention.

## 3. Architecture

| Layer | Role | Enforced by |
|---|---|---|
| Entry points (HTTP, WebSocket, RPC channel) | Authenticate, resolve the tenant, bind a `Scope` | The only place a `Context` is created for a request |
| Extensions (GraphQL, RPC, auth, PubSub) | Submit requests; never execute | Requests carry no actor or tenant |
| Scoped executor (`ash-core`) | Plan: tenancy, row and join policies, client-input rules, write and conflict guards, transactions | Only core can build an executable plan |
| Data layers | Execute plans | The `DataLayer` trait only accepts plans |
| Presentation (`ash-core`) | Build a per-reader `View`: field policies, embedded fields, derived values | Only `View`s reach serializers and events |

### Capabilities

| Capability | Created by | Grants | State |
|---|---|---|---|
| `Runtime` | `main`, once (`Runtime::new(data)`) | Creating everything below | Implemented |
| `Context` | `runtime.bind(actor, tenant)` | Acting as that actor in that tenant | Implemented |
| System scope | `runtime.system().bind(actor, tenant)` | Acting without authorization (seeds, migrations, jobs), still within tenancy | Implemented |
| `CredentialReadPermit` | `runtime.credential_reads::<R>()` | Reading credentials on one resource without read policies, never without tenancy | Implemented |
| Cross-tenant lookup grant | Planned; declared on one resource and action | Finding which tenant a record belongs to; returns an id and tenant, never records | Planned |
| Multi-scope transaction | `runtime.transaction(...)` | Several bound scopes in one transaction | Implemented |

Request data (bodies, headers, cursors, subscription variables) can never become one of these. Code holding only a `Context` can't create the others.

What this doesn't protect against: trusted server code holding a `Runtime` can still bind the wrong tenant on purpose. Privileged acts are visible calls, so they're easy to find and review, but no type system can judge intent.

## 4. Departures from Elixir Ash

This table describes the target design. §5 says which parts are implemented.

| Concern | Elixir Ash (documented behaviour) | ash-rust | Why | Cost |
|---|---|---|---|---|
| Actor and tenant | Options on each call; `Ash.Scope` groups them; preparations can call `set_tenant` | Bound once into a sealed `Scope` at the entry point. A `Context` can't change it | Callers can't forget or override it | A request handler can't switch tenant; jobs bind a new `Context` per tenant |
| Skipping authorization | `authorize?: false` on any call | `runtime.system()`, available only from the `Runtime` | Privileged code is visible and greppable | Hooks need a declared system capability |
| Cross-tenant reads | `multitenancy :bypass` / `:bypass_all` read actions, `global?` | A cross-tenant lookup grant per resource and action, returning only an id and tenant; resource-level `global` kept | Covers login-by-email without a general bypass | Bypass-style listing across tenants needs a system scope per tenant |
| Reaching the data layer | `Ash.DataLayer` callbacks are public; extensions and manual actions can build and run queries | Data layers accept only executable plans built by core. Apps keep direct access to their own database pool, outside the framework's guarantees | Tenancy and policies can't be skipped by building a query by hand | Data-layer-specific features need trusted plan nodes (`fragment`, full-text search, `for_update`) |
| Client input vs trusted expressions | `filter_input`/`sort_input` mark refs as input; `filter`/`sort` are trusted; the caller chooses | Separate `ClientExpr` and `TrustedExpr` types. Every extension parser produces `ClientExpr` | The compiler enforces the choice | An explicit conversion is needed to trust client input deliberately |
| Field-policy output | Read cleanup replaces hidden values with `%Ash.ForbiddenField{}` | Every output, including write results, bulk, manual persist and embedded resources, goes through one `View` builder | One code path instead of one per output | Raw records are internal types; manual persist returns them to core for presentation |
| Calculation inputs | Not authorized, by design (documented) | A calculation using a hidden field returns null unless it declares `declassify` | Derived values can't leak hidden inputs by accident | Patterns like `ssn_last_4` need an explicit declaration |
| `first` / `sum` aggregates | Aggregate the field's stored values | Aggregate only values the reader can see | Same reason | Results can differ between readers |
| Keyset pagination | Cursor values encode the sort values | Cursors are encrypted and authenticated by the `Runtime` | Clients can't read or forge sort values | Configure a shared Runtime key in production; rotating it invalidates cursors. Authenticated cursor values are trusted, so hidden prepared sorts remain pageable |
| PubSub | Notifications carry the record, changeset and actor; not authorized per subscriber | Raw events stay internal. Subscribers get `View`s for their own scope. Audit, mail and cache consumers register privileged sinks at startup | Payloads can't leak fields or tenants by default | Consumers that want raw data must be declared |
| Subscription deduplication | An `actor` function can map subscribers to a shared actor | Groups are keyed on a whole bound `Scope` and resolved *as* that group's actor | Sharing is safe because results are computed for the group | Groups must be expressible as an actor |
| RPC tenant (vs AshTypescript) | An explicit tenant in the payload can be used | The payload's tenant is ignored; the adapter binds it | The server owns tenancy | Clients that switch tenant need a server route that rebinds |
| Session state (vs AshAuthentication) | Tokens in a token resource, with revocation checks, optional token-presence requirement | Durable revocation, conditional consume (one `UPDATE ... WHERE consumed = false`), and a credential version bumped whenever the password hash changes | Logout and resets hold across instances, restarts and races | One storage read per authenticated request |
| Transactions across tenants | Pass different `tenant:` options inside `Ash.transact` | `runtime.transaction` holding several bound scopes; each operation names its scope | Cross-tenant work stays explicit | Only the `Runtime` can start one |

### Unchanged

Resources, actions, changes, validations, preparations, policies and field policies keep Ash's semantics. Multitenancy keeps the attribute and context (schema-per-tenant) strategies and `global`. A hidden field still appears as "forbidden" rather than missing.

## 5. Status

| Part | Step | State |
|---|---|---|
| Scoped reads and credential grants for authentication; RPC ignores the payload tenant | 1 | In review (#86) |
| `Runtime`, sealed `Scope`, `bind`/`system`, private `Context` fields, changesets tied to their scope, multi-scope `runtime.transaction`, every transport (HTTP and both WebSocket protocols) bound through the same binder | 2 | In review (#87) |
| Cross-tenant lookup grant | — | Planned. Until it lands, apps that log in by email across tenants (such as the Lagrange example) ask for the tenant first |
| System capabilities declared by action hooks | — | Planned |
| `ClientExpr`/`TrustedExpr`, destination and join traversal scopes, Runtime-sealed cursors, snapshot filter registration checks | 3 | In review (#89). See [Client read plans](../client-read-plans.md) |
| Recursive `View` presentation for reads, single/atomic/bulk/manual writes and resource-valued actions; non-serializable hook records; GraphQL masks and RPC selections | 4 | In review (#88). Typed APIs return projected resource structs; raw data-layer transport remains until step 8 |
| Derived-value visibility and `declassify` | 5 | In review (#90). Calculation dependencies and explicit policies; projected first/sum operands and client comparisons; declared custom inputs. See [Derived values](../scoped-access.md#derived-values) |
| Core-owned upsert plans; tenant, write-policy and version guards on single/bulk conflicts; scoped returns; memory no-change semantics | 6 | In review (#91). See [Guarded upsert conflicts](../scoped-access.md#guarded-upsert-conflicts) |
| Data-layer-specific trusted plan nodes | — | Deferred; trusted expression extension point retained |
| Managed relationships through scoped destination and join actions; scoped cascade discovery; required transactions and memory snapshot commits | 7 | Implemented, pending review. See [Managed relationships and cascades](../scoped-access.md#managed-relationships-and-cascades) |
| Delete raw data-layer access (bypasses become compile errors) | 8 | Planned |
| Private events; scoped subscriptions, with deduplicated groups keyed on a bound `Scope` | 9 | Planned |
| Durable, atomic session state | 10 | Planned |

Until step 8, raw data-layer access (`Context::data` and `ash_core::insert`) still exists, deprecated, so the guarantees are conventions that migrated code follows. Step 8 removes it.
