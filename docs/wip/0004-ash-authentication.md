# RFC 0004: `ash-authentication` — Declarative Authentication Engine for `ash-rust`

- **Status**: Proposed / WIP
- **Date**: 2026-09-04
- **Authors**: Will, Ash-Rust Team
- **Target Crates**: `ash-authentication` (new crate), `ash-authentication-macros` (new crate), `ash-core`, `ash-graphql` (optional integration)

---

## 1. Executive Summary

This RFC specifies the architecture, DSL design, cryptographic primitives, and HTTP integration of **`ash-authentication`**, a modular extension crate bringing first-class authentication to `ash-rust`.

Modeled after **`ash_authentication`** in the Elixir Ash ecosystem, `ash-authentication` eliminates hand-rolled cryptography, ad-hoc session tracking, and bespoke user sign-in handlers by transforming authentication into **declarative resource strategies**:
1. **Password Strategy**: Argon2id password hashing, password confirmation checks, and action lifecycles (`register_with_password`, `sign_in_with_password`, `change_password`).
2. **Token & JWT Service**: Cryptographically signed bearer tokens (HMAC-SHA256 via `jsonwebtoken`), configurable expiration, namespaced claims, and persisted token revocation.
3. **API Key Strategy**: Unsalted SHA-256 hashes of high-entropy random API keys for machine-to-machine authentication. The configured prefix applies to generation and parsing. Expensive password hashing is unnecessary for these randomly generated secrets.
4. **Declarative Transformer Macro (`#[authentication]`)**: Injects required authentication fields (e.g. `hashed_password`), registers validation changes, and sets up authentication actions with zero boilerplate.
5. **Axum HTTP Extractor & Authentication Router**: `AuthUser` extracts a `BoundIdentity` through `RequestAuthenticator`. The router provides sign-in, refresh, revoke, password lifecycle and `/me` endpoints. Registration uses resource actions, not a built-in HTTP route.
6. **Zero-Bypass Policy Integration**: Seamlessly maps authenticated identities into `ash_core::Actor`, guaranteeing all downstream resource actions and field policies enforce authorization transparently.

---

## 2. Motivation & Ash Elixir Parity

### The Problem in Traditional Rust Web Frameworks

In the Rust ecosystem today (using Axum, Actix, or Tower), implementing user authentication requires significant manual glue code:
- Developers must manually wire password hashing libraries (Argon2, bcrypt) into request DTOs.
- Password hashing is frequently omitted during seed scripts or updates, leading to plaintext leaks or security vulnerabilities.
- Timing attacks can occur if credential checks fail with early returns before comparing password hashes.
- JWT decoding, claims validation, and database user lookup must be repeated across every protected route.
- Field security is often overlooked: developers forget to annotate database structs with `#[serde(skip_serializing)]` on password hashes, inadvertently exposing hashes over JSON or GraphQL endpoints.

### How Ash Solves This

In Elixir's Ash, **`ash_authentication`** treats authentication as an extension on top of standard resources:
- A `User` resource declares strategies: `password`, `tokens`, `api_key`.
- The DSL generates standard Ash actions (`register_with_password`, `sign_in_with_password`).
- Input arguments (`password`, `password_confirmation`) are validated, hashed via changeset hooks, and never stored directly.
- The `hashed_password` attribute is automatically marked private, preventing accidental data leaks.
- Successful authentication produces a verified token and populates the `Ash.Context.actor`.

`ash-authentication` brings this exact declarative model to Rust.

---

## 3. Core Architecture & Components

```text
 ┌────────────────────────────────────────────────────────┐
 │                    HTTP Client / UI                    │
 └───────────────────────────┬────────────────────────────┘
                             │
                  Bearer <jwt> / Credentials
                             │
 ┌───────────────────────────▼────────────────────────────┐
 │            ash-authentication Axum Router              │
 │  POST /auth/sign-in   POST /auth/refresh   GET /auth/me │
 └───────────────────────────┬────────────────────────────┘
                             │
              AuthUser / Persisted Authentication
                             │
 ┌───────────────────────────▼────────────────────────────┐
 │               ash-authentication Strategies            │
 │     PasswordStrategy      ApiKeyStrategy      JwtService│
 └───────────────────────────┬────────────────────────────┘
                             │
                 Binds identity through Runtime
                             │
 ┌───────────────────────────▼────────────────────────────┐
 │               ash-core Resource Pipeline               │
 │       Policies / Field Redaction / Relational Stores   │
 └────────────────────────────────────────────────────────┘
```

---

## 4. Detailed Specification

### 4.1 Cryptographic Primitives (`PasswordService`)

Passwords must be stored using modern memory-hard password hashing algorithms:
- **Default Algorithm**: **Argon2id** (Argon2 version 0x13) via the pure-Rust `argon2` crate.
- **Parameters**: The argon2 crate's defaults, 19 MiB memory, 2 iterations and 1 lane. These are not RFC 9106's recommended parameter sets.
- **Verification**: The argon2 crate verifies PHC password hashes. This does not make the entire sign-in or reset HTTP request constant time.
- **Salt Generation**: Cryptographically secure random salts (16 bytes) generated via `getrandom`.

### 4.2 Token Management (`JwtService`)

Bearer authentication uses persisted session and subject state:
- **Algorithm**: `HS256` (HMAC-SHA256) signed by a configurable secret key.
- **Standard Claims**:
  ```rust
  #[derive(Clone, Debug, Serialize, Deserialize)]
  pub struct Claims {
      pub sub: Uuid,           // Subject (User ID)
      pub resource: String,    // Subject resource namespace
      pub credential_version: i64,
      pub exp: u64,            // Expiration timestamp
      pub iat: u64,            // Issued-at timestamp
      pub jti: String,         // Unique JWT token identifier
      pub purpose: String,    // access, refresh, password_reset, etc.
      pub role: Option<String>,
      pub tenant: Option<String>,
  }
  ```
- **Persisted authentication**: `JwtService::verify_token` checks cryptography and
  requires `exp > now`. `AuthStrategy::authenticate` is async and checks the resolved tenant,
  persisted session state, live subject and credential version in one required
  backend transaction. Uncoordinated subject/token stores are rejected. It returns a
  `BoundIdentity` carrying actor and tenant together.
- **Existing tokens**: missing resource/version claims are rejected immediately.
  There is no compatibility window or local acceptance cache. All newly issued
  access, refresh and reset tokens need a persisted row.

### 4.3 Declarative Resource DSL (`#[authentication]`)

Using Ash-Rust's Pattern 2 (Transformative Macro Decorator):

```rust
use ash_authentication::authentication;

#[authentication]
resource! {
    resource User;
    table "users";

    attributes {
        id: Uuid [pk],
        email: String,
        // `hashed_password` injected automatically!
    }

    authentication {
        strategy password {
            identity_field: email;
            hashed_password_field: hashed_password;
            hash_algorithm: "argon2id";
            min_password_length: 8;
        }

        strategy tokens {
            token_lifetime_secs: 3600;
        }
    }

    actions {
        read read { primary; }
        // `register_with_password` and `sign_in_with_password` injected automatically!
    }
}
```

The transformer automatically:
1. Injects `hashed_password: Option<String>` (or `String`) into `attributes`.
2. Generates the `register_with_password` create action accepting `email`, `password`, and `password_confirmation`, attaching `HashPasswordChange`.
3. Injects field policies redacting `hashed_password` on public reads.
4. Generates typed helpers `User::register_with_password` and `User::sign_in_with_password`.

### 4.4 Database-backed session state

`AshToken` stores `id`, `resource`, `tenant_key`, `jti`, `subject`, `purpose`,
`expires_at`, `revoked`, `consumed` and optional `extra`. A unique identity covers
`(resource, tenant_key, subject, jti)`. The tenant key encodes `Option<&str>` as JSON,
so absence never equals a tenant string. Resource names must be unique in a store.

All JWT sessions are persisted. Every authentication reads that authoritative
state and the subject's credential version. A different Runtime or restarted
instance sees revocation through the shared database. The synchronous in-memory
revocation traits and JwtService revocation methods are removed.

Refresh/reset consumption conditionally sets `consumed = true` only for a matching
namespace, purpose, subject, unrevoked/unconsumed state and unexpired row. Exactly
one returned row is success. Memory performs compare-and-set under its write lock
and validates snapshot commits. SQLite uses its writer lock, and Postgres uses
row locking in the conditional update. Refresh rotation consumes the old token
and persists both successor tokens in the same required transaction.

`DatabaseTokenStore::prune_expired` removes records with `expires_at <= now` in its
resource/tenant namespace. Issuance and revocation call it automatically. Schedule
it for idle namespaces that need cleanup without token writes. Revocation preserves
token expiry; unknown raw JTIs create no tombstones. Trusted server code can call
`AuthStrategy::revoke_jti` without a signer to revoke existing state. This API is
not exposed on the public router.

### 4.5 Password lifecycle

The macro injects `credential_version: i64 = 0` and declares a resource-level
credential version rule. A stored password-hash change increments the version in
the same write on memory, SQLite and Postgres. The rule also applies to app/admin
actions, bulk updates, direct hash writes and upserts. The version is sensitive
storage state and cannot be assigned by action input. Authentication rejects old
versions, and a deleted subject no longer authenticates.

Change-password verifies the current hash and validates/hashes the replacement
before its transaction. The update compares the stored hash, changes hash/version
and revokes prior JWT sessions together. Reset validates/hashes first, then consumes
the token, checks the live subject/version, updates hash/version and revokes prior
sessions in one required transaction. Revocation errors propagate and roll back.

API keys remain valid across password changes and resets by explicit policy.
Clearing or replacing the API-key hash revokes them. No API-key expiry or separate
key lifecycle is introduced here.

The subject and token store must share one transactional backend. A split store or
`StoreRegistry` without a consistency protocol is rejected with
`AtomicityUnsupported`. Authentication may bind its resulting identity to a registry
afterward. Reset requests commit their token before sending mail and reject an
outer transaction that could later roll back. Sender errors occur after commit.
The public reset-request endpoint always returns 200 with
`{"status":"reset_requested"}` for valid JSON, including unknown identities and
operational failures. Trusted strategy callers still receive errors. Both account
paths perform the scoped lookup, random JWT generation and signature verification;
neither hashes a password. Unknown identities persist nothing and send no mail.
Database writes, commit and mail latency still differ, so request timing is not
constant. Queue-backed senders and rate limits belong at the application boundary.

Schema upgrades, the immediate rejection of older JWTs and the changed extractor,
signer and token-store APIs are described in the
[authentication migration guide](../../crates/ash-authentication/README.md#migrate-existing-installations).

### 4.6 Axum Web Integration

With the `axum` feature enabled:
```rust
let strategy = User::auth_strategy(&runtime).with_jwt_service((*jwt_service).clone());
let auth_routes = auth_router(strategy.clone(), jwt_service.clone(), ctx.clone());
let authenticator = ash_authentication::RequestAuthenticator::new(strategy, ctx);

let app = Router::new()
    .nest("/auth", auth_routes)
    .route("/api/profile", get(profile_handler))
    .with_state(authenticator);

async fn profile_handler(AuthUser(identity): AuthUser) -> Json<serde_json::Value> {
    // Identity includes the live actor and resolved tenant.
    Json(serde_json::json!({ "user_id": identity.actor().id, "role": identity.actor().role() }))
}
```

#### Standard Endpoints

| Method | Path | Description | Authentication |
|---|---|---|---|
| `POST` | `/auth/sign-in` | Authenticate with credentials, return access + refresh tokens | Public |
| `POST` | `/auth/refresh` | Rotate refresh token, return new token pair | Public (refresh token) |
| `POST` | `/auth/revoke` | `{"token":"<JWT>"}` revokes itself; `{"all_sessions":true}` revokes the owner's JWT sessions | Signed token / persisted bearer access session |
| `POST` | `/auth/change-password` | Change user password | Bearer `<token>` |
| `POST` | `/auth/request-password-reset` | Request single-use password reset token | Public |
| `POST` | `/auth/reset-password` | Complete password reset with token | Public (reset token) |
| `GET` | `/auth/me` | Inspect the live authenticated identity | Bearer `<token>` |

Revoke checks signature, expiry and exact resource/tenant for self-revocation.
Owner revocation authenticates a live access session and accepts no subject or JTI
input. Invalid credentials return 401, unmatched/extra body fields return 422, and
success returns 200 with `{"status":"revoked"}`. Repeating an unexpired signed
token is idempotent. API keys and other subjects/tenants remain valid.

Breaking changes: raw JTIs are no longer accepted by `revoke_token` or HTTP revoke;
`RevokeTokenRequest` is an enum; tokens are rejected during their expiry second;
reset-request operational errors are hidden by the
generic HTTP response; configured API-key prefixes and payload format are enforced.
The [migration guide](../../crates/ash-authentication/README.md#migrate-existing-installations)
covers server-side JTI calls, Rust DTO construction, legacy tombstone cleanup and
reissuing keys generated under a previously ignored prefix. Keys contain the
configured prefix and 64 hex characters from two UUIDv4 values, with 244 random
bits. SHA-256 covers the entire raw key without salt.

---

## 5. Security & Safety Checklist

- [x] **No Plaintext Passwords**: Passwords never enter the database; hashed immediately in changeset before persistence.
- [x] **Password verification**: Uses the argon2 crate's verifier; full request timing can vary.
- [x] **Field Policy Redaction**: Password hashes are shielded from API serializers by default.
- [ ] **Secret key hygiene**: Applications must supply a high-entropy signing secret. `JwtService::new` does not enforce secret strength.
- [x] **Zero Memory-Unsafe Dependencies**: Pure Rust cryptography (`argon2`, `jsonwebtoken`).


Active subscriptions retain their authenticated context until they end. New HTTP
requests and subscription operations use async authentication, but automatic
termination or revalidation of existing streams on revocation or expiry is deferred.
