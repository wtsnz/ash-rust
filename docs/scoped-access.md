---
read_when:
  - Working on authentication tenant scoping or RPC request context
  - Migrating direct Context.data reads into the core engine
---

# Scoped access

`Context::access()` returns a borrowed `ScopedAccess` handle. Its reads use the
context's actor and tenant. `ReadRequest` supplies only filter, sort, limit and
offset, so it cannot replace execution identity.

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

These reads return raw `FieldMap`s. Apply field policies before exposing them in a
response. `ReadRequest` accepts trusted `Filter` and `Sort` values. Adapters still
must guard client expressions with the existing input policy helpers.

## Credential reads

Authentication needs password and API-key hashes before a user can pass ordinary
read policies. Trusted server configuration can create
`CredentialReadPermit::new(&User::DEF)`. The permit grants credential reads of that
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

Permit construction remains public in this first migration step. Core and the
separate authentication crate need a shared server bootstrap authority before grant
issuance can be restricted. That belongs to proposal step 2. A marker trait or a
public factory elsewhere would not restrict trusted Rust callers, which can still
access `Context::data` directly.

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
headers or channel connection credentials to select tenancy. GraphQL continues to
use the context its server adapter supplies.

## Limits

This completes step 1 of the scoped-executor proposal, not the entire migration.
`Context::data`, actor and tenant fields remain public, and other consumers can
bypass this handle. There is no immutable Runtime/Scope binding, opaque executable
plan or reader-specific View yet. Credential grants and results are not sealed.

This change does not fix token namespace isolation, durable access revocation,
deleted-user validation, session invalidation or atomic reset/refresh consumption.
It also does not fix managed relationships, upsert conflicts, derived or embedded
field policies, or subscription authorization and payloads. The audit reports and
full migration sequence remain in the planning workspace at
`ISSUES/ash-rust/ARCHITECTURE_PROPOSAL.md`.

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
`tenant_binding_postgres.rs`; RPC regressions are in `rpc_scope.rs`. The inconsistent
reset-claim fixtures sign and store their tokens directly because tenant-scoped
reset requests can no longer issue those credentials.
