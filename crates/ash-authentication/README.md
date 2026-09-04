# ash-authentication

Declarative authentication engine for [`ash-rust`](https://github.com/will/ash-rust), bringing Elixir's `ash_authentication` model to Rust with Argon2id password hashing, database-backed `Token` resources, refresh token rotation, JWT token services, API key management, and Axum HTTP extractors.

---

## Features

- 🔒 **Argon2id Password Strategy**: Memory-hard password hashing with RFC 9106 recommended defaults, constant-time verification, password confirmation checking, password change, and single-use password reset flows.
- 🔄 **Refresh Token Rotation & `AshToken` Resource**: Persistent database-backed token storage (`AshToken`), single-use refresh token rotation preventing replay attacks, and expired token pruning.
- 🎟️ **JWT Token Service**: Cryptographically signed bearer tokens (`HS256` via `jsonwebtoken`), token pairs (`TokenPair`), token revocation tracking (`TokenRevocationStore`), and claims mapping.
- 🔑 **API Key Strategy**: Salted SHA-256 tokens for machine-to-machine service accounts with constant-time equality checks.
- 🪄 **Macro Transformer (`#[authentication]`)**: Automatically injects `hashed_password`, registration actions, and password hashing changeset hooks into `ash_core::resource!`.
- 🚀 **Axum HTTP Integration**:
  - `AuthUser` extractor for typed actor extraction in handlers.
  - Pre-built `/auth` router:
    - `POST /auth/sign-in`: Returns access and refresh token pair.
    - `POST /auth/refresh`: Rotates refresh token, issuing a new token pair and revoking the old one.
    - `POST /auth/revoke`: Invalidates an active token or session.
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
    resource User;
    table "users";

    attributes {
        id: Uuid [pk],
        email: String,
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
```

### 2. User Registration, Sign-In & Token Rotation

```rust
let ctx = Context::new(Memory::new());

// 1. Register with password confirmation
let user = User::register_with_password(&ctx)
    .email("alice@example.com")
    .password("super-secret-password-123")
    .password_confirmation("super-secret-password-123")
    .await?;

// 2. Sign in with password and receive access + refresh token pair
let strategy = User::auth_strategy().with_jwt_service(jwt_service);
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
let auth_routes = auth_router(User::auth_strategy(), jwt_service.clone(), ctx);

let app = Router::new()
    .nest("/auth", auth_routes)
    .route("/api/me", get(profile_handler))
    .with_state(AppState { jwt: jwt_service });

async fn profile_handler(AuthUser(actor): AuthUser) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "user_id": actor.id,
        "role": actor.role(),
    }))
}
```

---

## Running Example

```bash
cargo run --example auth_server --features axum -p ash-authentication
```
