use std::sync::Arc;

use ash_authentication::{JwtService, authentication};
#[cfg(feature = "axum")]
use ash_authentication::{AuthUser, auth_router};
use ash_core::{Context, Result, Value};
use ash_memory::Memory;
use uuid::Uuid;

#[authentication]
ash_core::resource! {
    resource User;
    table "users";

    attributes {
        id: Uuid [pk],
        email: String,
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
            key_prefix: "ash_test_";
        }
    }

    actions {
        read read { primary; }
        update update {
            primary;
            accept { api_key_hash: Option<String> }
        }
    }
}

#[tokio::test]
async fn test_declarative_authentication_registration_and_sign_in() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // 1. Register a new user using the injected `register_with_password` action
    let user: User = User::register_with_password(&ctx)
        .email("alice@example.com")
        .password("super-secret-password-123")
        .password_confirmation("super-secret-password-123")
        .await
        .expect("registration should succeed");

    assert_eq!(user.email, "alice@example.com");
    assert!(user.hashed_password.is_some());
    let hash = user.hashed_password.as_ref().unwrap();
    assert!(hash.starts_with("$argon2id$"));

    // 2. Test successful sign in with password strategy
    let strategy = User::auth_strategy();
    let auth_user = strategy
        .sign_in_with_password(&ctx, "alice@example.com", "super-secret-password-123")
        .await
        .expect("sign in should succeed");
    assert_eq!(auth_user.id, user.id);

    // 3. Test failed sign in with incorrect password
    let failed_auth = strategy
        .sign_in_with_password(&ctx, "alice@example.com", "wrong-password")
        .await;
    assert!(failed_auth.is_err());

    // 4. Test failed registration with password mismatch
    let mismatch_err = User::register_with_password(&ctx)
        .email("bob@example.com")
        .password("super-secret-password-123")
        .password_confirmation("different-password")
        .await;
    assert!(mismatch_err.is_err());

    // 5. Test failed registration with password too short
    let short_err = User::register_with_password(&ctx)
        .email("charlie@example.com")
        .password("short")
        .password_confirmation("short")
        .await;
    assert!(short_err.is_err());

    Ok(())
}

#[tokio::test]
async fn test_api_key_authentication() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let strategy = User::auth_strategy();
    let api_key_service = ash_authentication::ApiKeyService::new("ash_test_");
    let (raw_key, key_hash) = api_key_service.generate_api_key();

    // Create user with API key hash
    let user: User = User::register_with_password(&ctx)
        .email("robot@service.com")
        .password("service-account-password-123")
        .password_confirmation("service-account-password-123")
        .await
        .expect("service user registration should succeed");

    // Update with api_key_hash
    let mut fields = std::collections::HashMap::new();
    fields.insert("api_key_hash".to_string(), Value::String(key_hash));
    let updated: User = ash_core::update::<User, _>(&ctx, "update", user.id, fields).await?;

    assert!(updated.api_key_hash.is_some());

    // Verify authentication by raw api key
    let verified = strategy
        .authenticate_api_key(&ctx, &raw_key)
        .await
        .expect("api key authentication should succeed");
    assert_eq!(verified.id, user.id);

    // Invalid api key should fail
    let invalid = strategy.authenticate_api_key(&ctx, "invalid_api_key").await;
    assert!(invalid.is_err());

    Ok(())
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn test_axum_auth_router_and_auth_user_extractor() -> Result<()> {
    use axum::http::StatusCode;
    use axum::routing::get;
    use axum::{Json, Router};
    use serde_json::json;
    use tower::ServiceExt;

    let ctx = Context::new(Memory::new());
    let jwt_service = Arc::new(JwtService::new(
        "test-super-secret-signing-key-for-jwt-tokens-123456",
    ));

    // Register user
    let user: User = User::register_with_password(&ctx)
        .email("alice_web@example.com")
        .password("strong-password-123")
        .password_confirmation("strong-password-123")
        .await
        .expect("web user registration should succeed");

    let strategy = User::auth_strategy();

    // App state holding JwtService for extractor
    #[derive(Clone)]
    struct AppState {
        jwt: Arc<JwtService>,
    }

    impl axum::extract::FromRef<AppState> for Arc<JwtService> {
        fn from_ref(state: &AppState) -> Self {
            state.jwt.clone()
        }
    }

    let app_state = AppState {
        jwt: jwt_service.clone(),
    };

    // Protected route using AuthUser extractor
    async fn protected_handler(AuthUser(actor): AuthUser) -> Json<serde_json::Value> {
        Json(json!({
            "status": "ok",
            "actor_id": actor.id.to_string(),
            "role": actor.role(),
        }))
    }

    let auth_routes = auth_router(strategy, jwt_service.clone(), ctx);

    let app: Router = Router::new()
        .nest("/auth", auth_routes)
        .route("/protected", get(protected_handler))
        .with_state(app_state);

    // 1. Test POST /auth/sign-in
    let sign_in_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/sign-in")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "identity": "alice_web@example.com",
                    "password": "strong-password-123"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(sign_in_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let auth_res: ash_authentication::AuthTokenResponse =
        serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(auth_res.user_id, user.id);
    assert_eq!(auth_res.role.as_deref(), None);
    let token = auth_res.token;

    // 2. Test GET /protected with valid Bearer token
    let protected_req = axum::http::Request::builder()
        .method("GET")
        .uri("/protected")
        .header("Authorization", format!("Bearer {token}"))
        .body(http_body_util::BodyExt::boxed(http_body_util::Full::new(
            bytes::Bytes::new(),
        )))
        .unwrap();

    let res = app.clone().oneshot(protected_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let json_val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json_val["actor_id"], user.id.to_string());

    // 3. Test GET /protected with missing token
    let unauth_req = axum::http::Request::builder()
        .method("GET")
        .uri("/protected")
        .body(http_body_util::BodyExt::boxed(http_body_util::Full::new(
            bytes::Bytes::new(),
        )))
        .unwrap();

    let res = app.clone().oneshot(unauth_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 4. Test GET /auth/me
    let me_req = axum::http::Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header("Authorization", format!("Bearer {token}"))
        .body(http_body_util::BodyExt::boxed(http_body_util::Full::new(
            bytes::Bytes::new(),
        )))
        .unwrap();

    let res = app.oneshot(me_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let me_val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(me_val["user_id"], user.id.to_string());

    Ok(())
}
