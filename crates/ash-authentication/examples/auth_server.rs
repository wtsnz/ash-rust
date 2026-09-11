//! # Ash Authentication Server Example
//!
//! Run with:
//! ```bash
//! cargo run --example auth_server --features axum -p ash-authentication
//! ```

use std::sync::Arc;

use ash_authentication::{AuthUser, JwtService, auth_router, authentication};
use ash_core::{Context, Result};
use ash_memory::Memory;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;
use uuid::Uuid;

#[authentication]
ash_core::resource! {
    Account {
        table "accounts";

    attributes {
        id: Uuid [pk];
        email: String;
    }

    authentication {
        strategy password {
            identity_field: email;
            hashed_password_field: hashed_password;
            min_password_length: 8;
            require_confirmation: true;
        }

        strategy tokens {
            token_lifetime_secs: 7200;
        }
    }

    actions {
        read read { primary; }
    }
    }}

#[derive(Clone)]
struct AppState {
    jwt: Arc<JwtService>,
}

impl axum::extract::FromRef<AppState> for Arc<JwtService> {
    fn from_ref(state: &AppState) -> Self {
        state.jwt.clone()
    }
}

async fn protected_profile(AuthUser(actor): AuthUser) -> Json<serde_json::Value> {
    Json(json!({
        "status": "authenticated",
        "actor_id": actor.id.to_string(),
        "role": actor.role(),
    }))
}

#[tokio::main]
async fn main() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let jwt_service = Arc::new(JwtService::new(
        "super-secret-ash-jwt-signature-key-at-least-32-bytes-long",
    ));

    // Seed initial user:
    let account = Account::register_with_password(&ctx)
        .email("admin@ash-rust.org")
        .password("password123")
        .password_confirmation("password123")
        .await
        .expect("seed account");

    println!("Seeded account: {} ({})", account.email, account.id);

    let auth_routes = auth_router(Account::auth_strategy(), jwt_service.clone(), ctx);

    let app = Router::new()
        .nest("/auth", auth_routes)
        .route("/api/profile", get(protected_profile))
        .with_state(AppState {
            jwt: jwt_service.clone(),
        });

    println!("Ash Authentication Server running on http://127.0.0.1:3000");
    println!("Endpoints available:");
    println!("  • POST /auth/sign-in               - Sign in, receive access + refresh token pair");
    println!("  • POST /auth/refresh               - Rotate refresh token & receive new pair");
    println!("  • POST /auth/revoke                - Invalidate token");
    println!("  • POST /auth/change-password       - Change password (requires Bearer token)");
    println!("  • POST /auth/request-password-reset- Request single-use password reset token");
    println!("  • POST /auth/reset-password        - Reset password with single-use token");
    println!("  • GET  /auth/me                    - Inspect authenticated identity claims");
    println!("  • GET  /api/profile                - Protected application route");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
