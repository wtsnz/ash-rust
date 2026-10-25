# ash-authentication

Declarative authentication engine for [`ash-rust`](https://github.com/will/ash-rust), bringing Elixir's `ash_authentication` model to Rust with Argon2id password hashing, database-backed `Token` resources, refresh token rotation, JWT token services, API key management, and Axum HTTP extractors.

---

## Features

- 🔒 **Argon2id Password Strategy**: Memory-hard password hashing with the argon2 crate's defaults, password confirmation checking, password change, and single-use password reset flows.
- 🔄 **Refresh Token Rotation & `AshToken` Resource**: Persistent database-backed token storage (`AshToken`), single-use refresh token rotation preventing replay attacks, and expired token pruning.
- 🎟️ **JWT Token Service**: Cryptographically signed bearer tokens (`HS256` via `jsonwebtoken`), token pairs (`TokenPair`), persisted session revocation, and claims mapping.
- 🔑 **API Key Strategy**: SHA-256 hashes of high-entropy random keys for machine-to-machine service accounts, with configured prefixes and constant-time hash comparison.
- 🪄 **Macro Transformer (`#[authentication]`)**: Automatically injects `hashed_password`, registration actions, and password hashing changeset hooks into `ash_core::resource!`.
- 🚀 **Axum HTTP Integration**:
  - `AuthUser` extractor for a live actor bound to its tenant.
  - Pre-built `/auth` router:
    - `POST /auth/sign-in`: Returns access and refresh token pair.
    - `POST /auth/refresh`: Rotates refresh token, issuing a new token pair and revoking the old one.
    - `POST /auth/revoke`: Revokes a signed token itself, or all sessions of an authenticated owner.
    - `POST /auth/change-password`: Verifies current password, validates new password, and updates hash.
    - `POST /auth/request-password-reset`: Generates a single-use password reset token.
    - `POST /auth/reset-password`: Consumes the reset token and securely sets the new password.
    - `GET  /auth/me`: Inspects the authenticated actor's claims.
- 🛡️ **Zero-Bypass Policy Integration**: Seamlessly maps authenticated credentials into `ash_core::Actor` to enforce all resource and field policies.

---

## Quick Start

### 1. Declare an Authenticated Resource

```rust
use ash_authentication::authentication;

#[authentication]
ash_core::resource! {
    User {
        table "users";

        attributes {
            id: Uuid [pk];
            email: String;
            // `hashed_password: Option<String>` injected automatically!
        }

        authentication {
            strategy password {
                identity_field: email;
                hashed_password_field: hashed_password;
                min_password_length: 8;
                require_confirmation: true;
            }

            strategy tokens {
                token_lifetime_secs: 3600;
            }

            strategy api_key {
                api_key_field: api_key_hash;
                key_prefix: "ash_live_";
            }
        }

        actions {
            read read { primary; }
            // `register_with_password` action injected automatically!
        }
    }
}
```

### 2. User Registration, Sign-In & Token Rotation

```rust
let runtime = ash_core::Runtime::new(Memory::new());
let ctx = runtime.bind(None, None);

// 1. Register with password confirmation
let user = User::register_with_password(&ctx)
    .email("alice@example.com")
    .password("super-secret-password-123")
    .password_confirmation("super-secret-password-123")
    .await?;

// 2. Sign in with password and receive access + refresh token pair
let strategy = User::auth_strategy(&runtime).with_jwt_service(jwt_service);
let (auth_user, pair) = strategy
    .sign_in_with_password_and_token_pair(&ctx, "alice@example.com", "super-secret-password-123")
    .await?;

println!("Access token: {}", pair.access_token);
println!("Refresh token: {}", pair.refresh_token);

// 3. Rotate refresh token (old refresh token is immediately invalidated)
let (refreshed_user, new_pair) = strategy
    .rotate_refresh_token(&ctx, &pair.refresh_token)
    .await?;
```

### 3. Password Lifecycle: Change & Reset

```rust
// Change password (verifies current password in constant time)
strategy.change_password(
    &ctx,
    user.id,
    "old-password-123",
    "new-password-456",
    "new-password-456",
).await?;

// Password reset flow
let (user, reset_token) = strategy.request_password_reset(&ctx, "alice@example.com").await?;
strategy.reset_password_with_token(&ctx, &reset_token, "brand-new-pass", "brand-new-pass").await?;
```

### 4. Axum HTTP Integration

```rust
use ash_authentication::{AuthUser, JwtService, auth_router};
use axum::{Router, Json, routing::get};
use std::sync::Arc;

let jwt_service = Arc::new(JwtService::new("my-32-byte-secret-signing-key"));
let strategy = User::auth_strategy(&runtime).with_jwt_service((*jwt_service).clone());
let auth_routes = auth_router(strategy.clone(), jwt_service.clone(), ctx.clone());
let authenticator = ash_authentication::RequestAuthenticator::new(strategy, ctx);

let app = Router::new()
    .nest("/auth", auth_routes)
    .route("/api/me", get(profile_handler))
    .with_state(authenticator);

async fn profile_handler(AuthUser(identity): AuthUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "user_id": identity.actor().id,
        "role": identity.actor().role(),
    }))
}
```

---

## Running Example

```bash
cargo run --example auth_server --features axum -p ash-authentication
```

## Credential presentation

Configured password and API-key hash attributes are sensitive storage fields.
Ordinary action results and reads hide them, and generated GraphQL/TypeScript
schemas omit them. Generated SDK selection builders also omit sensitive storage
attributes so their default queries match the public GraphQL schema. This is a
breaking change for callers that inspected returned hashes. Use a Runtime-issued credential read permit in trusted authentication code.
Registration notifications contain no password arguments or automatic metadata.
See [Events and subscriptions](../../docs/scoped-access.md#events-and-subscriptions).


## Durable sessions

`JwtService::verify_token` and `verify_token_with_purpose` perform cryptographic
verification only. They check the signature, expiry with zero clock leeway, and
requested purpose. They do not authenticate a request.

Use `strategy.authenticate(&context, token).await` after resolving the request's
tenant. Its reads use one required backend transaction, rejecting uncoordinated
stores before returning an identity. Authentication requires a persisted, unrevoked, unconsumed session, a live
subject in that tenant, and the current credential version. It returns
`BoundIdentity`, which keeps the actor and tenant together. Use
`identity.bind(&runtime)` to bind subsequent execution. The actor comes from the
live subject, so role updates apply to the next authenticated request.

`AuthUser` now contains `BoundIdentity`. Its Axum state dependency is
`RequestAuthenticator`, which holds the strategy and a context with the resolved
tenant. Applications with dynamic tenants can call `AuthStrategy::authenticate`
from their own async extractor. A bare `Arc<JwtService>` no longer supports the
extractor. The standard router uses the same async decision for `/me` and
`/change-password`.

All issued access, refresh and reset JWTs have a persisted `AshToken` row.
The unique key is `(resource, tenant_key, subject, jti)`. `resource` is the subject's
`ResourceDef::name`. Keep resource names unique within a token store. `tenant_key`
is the JSON encoding of the optional tenant: no tenant is `null`, while a tenant
named `null` is the JSON string `"null"`. Empty strings remain distinct too.
Missing state rejects the token. No local acceptance cache or local revocation
store participates in authentication. Revocation therefore applies to every
Runtime using that database, including a new Runtime after restart.

The authentication macro injects `credential_version: i64 = 0` when absent. Password resources
also declare a resource-level `CredentialVersionRule`. Every supported backend
increments the stored version when the stored hash changes, in the same statement
or memory write lock as that change. The rule covers credential grants, direct
hash changes, `HashPasswordChange`, app/admin actions, atomic actions, bulk actions
and upsert conflicts. Supplying a version cannot override the rule. Writing the
same hash preserves the version. Tokens carry the version at issuance, and older
versions fail authentication even if their token rows remain present. Deleting a
subject also rejects its tokens.

API keys have an independent lifecycle. Password changes and resets deliberately
preserve them. Clearing or replacing the configured API-key hash revokes the old
key. API-key expiry and a separate key-revocation resource remain separate work.

Refresh and reset use one conditional write matching resource, exact tenant,
subject, JTI, purpose, `consumed = false`, `revoked = false` and a future expiry.
Exactly one returned row is success. Memory evaluates the write under its lock and
rejects conflicting snapshot commits. SQLite uses one conditional SQL update under
its writer lock. Postgres uses a conditional update with row locking. Rotation
consumes the old token and stores the successor pair in one required transaction.
Reset validates and hashes first, then consumes, updates the subject hash/version
and revokes prior sessions in one required transaction. Revocation or persistence
errors roll back the operation and are returned to the caller.

Subject and token state must use one transactional backend. `StoreRegistry` cannot
provide that guarantee and returns `AtomicityUnsupported` before JWT authentication
or token writes.
Keep authentication on the single subject backend, then bind its `BoundIdentity`
to a registry Runtime for ordinary resource execution. Separate token/subject
stores need a consistency protocol and are unsupported.

Reset requests commit the token before `AuthSender` sends mail. They reject an
outer transaction because its later rollback could invalidate an already sent
message. A sender failure is returned after commit; the stored token remains.
Other token operations can participate in a caller's required transaction.

## Public reset and revocation contracts

`POST /auth/request-password-reset` returns HTTP 200 and
`{"status":"reset_requested"}` for every valid request body, including unknown
accounts and persistence or sender failures. The strategy API still returns errors
to trusted callers. Only existing accounts receive mail, after token commit.
Both known and unknown identities perform the scoped lookup and generate and
verify a random reset JWT. No password is supplied, so neither path hashes a
password. Unknown-account JWTs are discarded without persistence or mail. This is
response uniformity, not a constant-time guarantee: lookup results, database writes,
commit and sender latency can still differ. Use a fast queue-backed sender and
request rate limits where timing exposure matters. Delivery failures need monitoring
in the application's sender; the HTTP response does not report delivery success.

`POST /auth/revoke` accepts exactly one of these JSON bodies:

- `{"token":"<JWT>"}` verifies signature and expiry, and revokes only that JWT in
  the router's resource and exact tenant. Access, refresh and reset tokens can
  revoke themselves. Repeating a still-unexpired JWT is idempotent. No bearer header
  is required because possession of the signed token authorizes self-revocation.
- `{"all_sessions":true}` requires `Authorization: Bearer <access JWT>`. The token
  must authenticate against persisted state and a live subject. Only that owner's
  JWT sessions in the resolved resource/tenant are revoked, in a required
  transaction. API keys are independent. No subject ID or raw JTI is accepted.

Invalid tokens or missing/invalid owner authentication return HTTP 401. JSON bodies
with extra fields or a shape that matches neither variant return HTTP 422.
Malformed JSON syntax uses Axum's HTTP 400 rejection. Success returns HTTP 200 and
`{"status":"revoked"}`. A verified but unpersisted JWT creates no state and
revokes no other session; expiry cleanup may still remove expired rows.

Revocation retains the original token expiry. Issuance and revocation automatically
call `DatabaseTokenStore::prune_expired`, removing rows with `expires_at <= now` in
the current resource/tenant namespace, including consumed and revoked rows. Cleanup
failure propagates. Schedule that method for each idle namespace if expired rows
must be removed while it has no token writes. Retention is until expiry plus the
next cleanup; there is no background timer. Trusted raw-JTI revocation updates
existing rows only, so unknown JTIs create no tombstones or permanent storage.

## API-key format and hashing

`AuthStrategy::api_key_service()` uses `ApiKeyStrategyDef::key_prefix` for generation
and parsing. The default is `ash_`. Keys have that prefix followed by 64 hex
characters from two random UUIDv4 values, providing 244 random bits. Authentication
requires this format and the configured prefix before looking up the hash.

The stored value is unsalted SHA-256 of the entire raw key, including the prefix.
These machine-generated secrets have enough entropy to resist offline guessing;
they do not need the salt and expensive password hashing required for human-chosen
passwords. Store only the hash and show the raw key once. `verify_api_key` checks
the format and compares the computed hash in constant time. Strategy authentication
uses a database hash lookup and does not claim constant-time request processing.

GraphQL's `RequestData::apply_async` and `connection_init_async` let adapters await
this authentication decision before binding HTTP requests or either WebSocket
protocol. Existing synchronous binders remain supported for already trusted
contexts. Lagrange uses the async hooks. Active subscriptions retain their bound
identity until they end; automatic termination or revalidation after logout,
password change, subject deletion or token expiry is deferred.

## Migrate existing installations

This is a breaking schema, claim and API change. Deploy the schema and all auth
instances together, and require users to sign in again. JWTs without both
`resource` and `credential_version` are rejected immediately. There is no legacy
acceptance window. Newly signed JWTs without a persisted row are also rejected.

1. Add `credential_version BIGINT NOT NULL DEFAULT 0` to each password/token subject
   table, including each schema for context-tenanted resources. SQLite uses INTEGER.
   The macro supplies this attribute for generated schemas. Manually configured
   password resources must declare it themselves, together with AuthenticationDef
   password metadata. Password operations reject a missing or mismatched credential
   version rule. Adding a version column alone does not enable password authentication.
2. Apply the backend-specific upgrade in [migrations](migrations). The upgrades
   discard legacy token state because its resource/tenant namespace cannot be
   reconstructed safely. Down migrations also invalidate sessions. If your token
   table has custom names or indexes, generate the corresponding migration from
   your resources with cargo-ash instead of applying the sample unchanged.
3. Install the current `AshToken::DEF` when creating a fresh database. Lagrange's
   committed cargo-ash migrations include both the subject version and token table.
4. Replace request-time cryptographic verification with async authentication.
   Replace `AuthUser(actor)` with `AuthUser(identity)` and read `identity.actor()`.
   Expose `RequestAuthenticator` through Axum state or `FromRef`.
5. Issue tokens through `AuthStrategy`. Low-level JWT signers now require resource
   and credential-version arguments. Signing alone does not persist a session.
   `DatabaseTokenStore::new` requires the subject resource name and a Runtime-issued
   `AshToken` write permit for `revoked` and `consumed`. Its token methods take
   `Claims`, retaining the complete namespace.

Step 11 adds these breaking changes:

- `AuthStrategy::revoke_token` accepts signed JWTs only. Trusted code that previously
  passed raw JTIs must use `AuthStrategy::revoke_jti` or
  `DatabaseTokenStore::revoke_jti`. These methods require no signer and no longer
  create tombstones for unknown JTIs. They do not preempt future token issuance.
- `RevokeTokenRequest` is now an enum. Replace Rust struct literals with
  `RevokeTokenRequest::Token { token }` or `OwnSessions { all_sessions: true }`.
  Existing HTTP `{"token":"<JWT>"}` bodies still work. Raw-JTI bodies now return
  401, and extra fields are rejected. Use the owner body and bearer header to log
  out all of an owner's sessions.
- Reset-request HTTP errors are replaced by the generic 200 response. Use the
  strategy or sender instrumentation for operational errors, and never interpret
  HTTP success as confirmation that an account exists or mail was delivered.
- Configured key prefixes and the 64-hex payload are now enforced. Installations
  that generated keys with an ignored prefix or stored arbitrary key formats must
  reissue keys using the configured service, or configure the prefix actually used
  by existing generated keys. Prefix changes require key reissuance.
- JWT verification now rejects `exp <= now`, matching persisted session expiry.
  It no longer accepts a token during its expiration second.

No schema or dependency change is required for step 11. Pruning also removes legacy
permanent tombstones identified by `purpose = 'revocation'`, a nil subject and
`expires_at = i64::MAX`, within the current namespace. These rows are unnecessary
because missing session state fails authentication. Run pruning once per existing
namespace during deployment, including namespaces with no new token traffic.

`MemoryRevocationStore`, `TokenRevocationStore` and the JwtService revocation
methods/configuration are removed. Use async strategy revocation backed by the
subject database. `TokenStrategyDef::track_revocations` remains as a legacy flag;
authentication always checks persisted revocation. Confirmation/reconfirmation,
refresh-family revocation, API-key expiry and a separate key-state resource remain
deferred, as does termination of active subscriptions.
