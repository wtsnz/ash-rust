# RFC 0004: `ash-authentication` — Declarative Authentication Engine for `ash-rust`

- **Status**: Proposed / WIP
- **Date**: 2026-09-04
- **Authors**: Will, Ash-Rust Team
- **Target Crates**: `ash-authentication` (new crate), `ash-authentication-macros` (new crate), `ash-core`, `ash-graphql` (optional integration)

---

## 1. Executive Summary

This RFC specifies the architecture, DSL design, cryptographic primitives, and HTTP integration of **`ash-authentication`**, a modular extension crate bringing first-class authentication to `ash-rust`.

Modeled after **`ash_authentication`** in the Elixir Ash ecosystem, `ash-authentication` eliminates hand-rolled cryptography, ad-hoc session tracking, and bespoke user sign-in handlers by transforming authentication into **declarative resource strategies**:
1. **Password Strategy**: Secure Argon2id password hashing, constant-time credential verification, password confirmation checks, and action lifecycles (`register_with_password`, `sign_in_with_password`, `change_password`).
2. **Token & JWT Service**: Cryptographically signed bearer tokens (HMAC-SHA256 via `jsonwebtoken`), configurable token expiration, payload claims (`sub`, `iss`, `aud`, `exp`, `jti`, roles), and token revocation tracking.
3. **API Key Strategy**: Salted SHA-256 API token generation and validation for machine-to-machine authentication.
4. **Declarative Transformer Macro (`#[authentication]`)**: Injects required authentication fields (e.g. `hashed_password`), registers validation changes, and sets up authentication actions with zero boilerplate.
5. **Axum HTTP Extractor & Authentication Router**: Pluggable `AuthUser<U>` extractor for Axum handlers, bearer token middleware, and automated endpoints (`/auth/register`, `/auth/sign-in`, `/auth/refresh`, `/auth/me`).
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
 │  POST /auth/register   POST /auth/sign-in   GET /auth/me │
 └───────────────────────────┬────────────────────────────┘
                             │
              AuthUser<User> / Token Verification
                             │
 ┌───────────────────────────▼────────────────────────────┐
 │               ash-authentication Strategies            │
 │     PasswordStrategy      ApiKeyStrategy      JwtService│
 └───────────────────────────┬────────────────────────────┘
                             │
                    Sets Context.actor
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
- **Parameters**: RFC 9106 recommended defaults (19 MiB memory, 2 iterations, 1 parallelism).
- **Constant-Time Verification**: Verification uses constant-time string comparison (`subtle` / constant-time equality) to protect against side-channel timing attacks.
- **Salt Generation**: Cryptographically secure random salts (16 bytes) generated via `getrandom`.

### 4.2 Token Management (`JwtService`)

Stateless or stateful bearer token authentication:
- **Algorithm**: `HS256` (HMAC-SHA256) signed by a configurable secret key.
- **Standard Claims**:
  ```rust
  #[derive(Clone, Debug, Serialize, Deserialize)]
  pub struct Claims {
      pub sub: Uuid,           // Subject (User ID)
      pub exp: usize,          // Expiration timestamp
      pub iat: usize,          // Issued-at timestamp
      pub jti: String,         // Unique JWT token identifier
      pub role: Option<String>,
      pub tenant: Option<String>,
      pub extra: FieldMap,
  }
  ```
- **Revocation Store (`TokenRevocationStore`)**:
  - Trait for tracking revoked tokens by `jti` or subject ID.
  - Built-in `MemoryRevocationStore` and support for database-backed revocation.

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

### 4.4 Axum Web Integration

With the `axum` feature enabled:
```rust
let auth_service = AuthService::new(jwt_service, user_def);

let app = Router::new()
    .nest("/auth", auth_service.into_router())
    .route("/api/profile", get(profile_handler));

async fn profile_handler(AuthUser(actor): AuthUser) -> Json<ProfileResponse> {
    // Actor is verified and loaded from Bearer token
    Json(ProfileResponse { user_id: actor.id })
}
```

---

## 5. Security & Safety Checklist

- [x] **No Plaintext Passwords**: Passwords never enter the database; hashed immediately in changeset before persistence.
- [x] **Timing-Attack Resistance**: Password comparison runs in constant-time.
- [x] **Field Policy Redaction**: Password hashes are shielded from API serializers by default.
- [x] **Secret Key Hygiene**: Enforces non-empty, high-entropy signing secrets for JWT tokens.
- [x] **Zero Memory-Unsafe Dependencies**: Pure Rust cryptography (`argon2`, `jsonwebtoken`).
