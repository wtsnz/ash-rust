#[cfg(feature = "axum")]
use std::sync::Arc;

use ash_authentication::{JwtService, authentication};
#[cfg(feature = "axum")]
use ash_authentication::{AuthUser, auth_router};
use ash_core::{Context, Resource, Result, Value};
use ash_memory::Memory;
use uuid::Uuid;

#[authentication]
ash_core::resource! {
    User {
        table "users";

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
            accept [api_key_hash];
        }
    }
    }}

#[tokio::test]
async fn test_declarative_authentication_registration_and_sign_in() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // Verify AuthenticationDef was injected into resource extensions
    let ext = User::DEF
        .extension::<ash_authentication::AuthenticationDef>()
        .expect("AuthenticationDef should be present on User::DEF");
    assert_eq!(
        ext.password.as_ref().unwrap().identity_field,
        "email"
    );
    assert_eq!(
        ext.password.as_ref().unwrap().min_password_length,
        8
    );

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

#[tokio::test]
async fn test_token_pair_and_database_token_store_rotation() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let jwt_service = JwtService::new("test-super-secret-rotation-key-1234567890");
    let strategy = User::auth_strategy().with_jwt_service(jwt_service);

    let _user: User = User::register_with_password(&ctx)
        .email("rotation_user@example.com")
        .password("rotation-password-123")
        .password_confirmation("rotation-password-123")
        .await
        .expect("registration should succeed");

    // 1. Sign in and receive TokenPair
    let (_auth_user, pair1) = strategy
        .sign_in_with_password_and_token_pair(
            &ctx,
            "rotation_user@example.com",
            "rotation-password-123",
        )
        .await
        .expect("sign-in with token pair should succeed");

    assert_eq!(pair1.token_type, "Bearer");
    assert!(!pair1.access_token.is_empty());
    assert!(!pair1.refresh_token.is_empty());

    // 2. Rotate refresh token -> issues new token pair
    let (_refreshed_user, pair2) = strategy
        .rotate_refresh_token(&ctx, &pair1.refresh_token)
        .await
        .expect("refresh token rotation should succeed");

    assert_ne!(pair1.access_token, pair2.access_token);
    assert_ne!(pair1.refresh_token, pair2.refresh_token);

    // 3. Replay attack prevention: old refresh token must fail
    let replay_err = strategy
        .rotate_refresh_token(&ctx, &pair1.refresh_token)
        .await;
    assert!(replay_err.is_err());

    // 4. Rotating with new refresh token succeeds again
    let (_refreshed_user2, pair3) = strategy
        .rotate_refresh_token(&ctx, &pair2.refresh_token)
        .await
        .expect("second refresh token rotation should succeed");
    assert_ne!(pair2.refresh_token, pair3.refresh_token);

    Ok(())
}

#[tokio::test]
async fn test_change_password_and_session_invalidation() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let jwt_service = JwtService::new("test-super-secret-password-change-key-123");
    let strategy = User::auth_strategy().with_jwt_service(jwt_service);

    let user: User = User::register_with_password(&ctx)
        .email("pwd_change@example.com")
        .password("old-password-123")
        .password_confirmation("old-password-123")
        .await
        .expect("registration should succeed");

    // Obtain token pair
    let (_, pair) = strategy
        .sign_in_with_password_and_token_pair(
            &ctx,
            "pwd_change@example.com",
            "old-password-123",
        )
        .await
        .expect("sign in should succeed");

    // 1. Change password with wrong current password should fail
    let wrong_pwd_err = strategy
        .change_password(
            &ctx,
            user.id,
            "wrong-current-password",
            "new-password-456",
            "new-password-456",
        )
        .await;
    assert!(wrong_pwd_err.is_err());

    // 2. Change password with confirmation mismatch should fail
    let mismatch_err = strategy
        .change_password(
            &ctx,
            user.id,
            "old-password-123",
            "new-password-456",
            "different-confirmation",
        )
        .await;
    assert!(mismatch_err.is_err());

    // 3. Successful change password
    strategy
        .change_password(
            &ctx,
            user.id,
            "old-password-123",
            "new-password-456",
            "new-password-456",
        )
        .await
        .expect("password change should succeed");

    // 4. Old password sign-in must fail
    let old_auth = strategy
        .sign_in_with_password(&ctx, "pwd_change@example.com", "old-password-123")
        .await;
    assert!(old_auth.is_err());

    // 5. New password sign-in succeeds
    let new_auth = strategy
        .sign_in_with_password(&ctx, "pwd_change@example.com", "new-password-456")
        .await
        .expect("sign in with new password should succeed");
    assert_eq!(new_auth.id, user.id);

    // 6. Previously active refresh token should now be revoked
    let refresh_err = strategy
        .rotate_refresh_token(&ctx, &pair.refresh_token)
        .await;
    assert!(refresh_err.is_err());

    Ok(())
}

#[tokio::test]
async fn test_password_reset_flow() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let jwt_service = JwtService::new("test-super-secret-password-reset-key-123");
    let strategy = User::auth_strategy().with_jwt_service(jwt_service);

    let user: User = User::register_with_password(&ctx)
        .email("reset_user@example.com")
        .password("original-pass-123")
        .password_confirmation("original-pass-123")
        .await
        .expect("registration should succeed");

    // 1. Request password reset
    let (_u, reset_token) = strategy
        .request_password_reset(&ctx, "reset_user@example.com")
        .await
        .expect("password reset request should succeed");

    // 2a. Password confirmation mismatch should fail and preserve reset token
    let mismatch_err = strategy
        .reset_password_with_token(
            &ctx,
            &reset_token,
            "brand-new-pass-999",
            "typo-in-password-confirmation",
        )
        .await;
    assert!(matches!(
        mismatch_err,
        Err(ash_authentication::AuthError::PasswordConfirmationMismatch)
    ));

    // 2b. Complete password reset with matching confirmation succeeds
    strategy
        .reset_password_with_token(
            &ctx,
            &reset_token,
            "brand-new-pass-999",
            "brand-new-pass-999",
        )
        .await
        .expect("password reset with token should succeed");

    // 3. Sign in with brand new password
    let auth = strategy
        .sign_in_with_password(&ctx, "reset_user@example.com", "brand-new-pass-999")
        .await
        .expect("sign in with reset password should succeed");
    assert_eq!(auth.id, user.id);

    // 4. Reset token cannot be reused (single-use token protection!)
    let reuse_err = strategy
        .reset_password_with_token(
            &ctx,
            &reset_token,
            "yet-another-pass-000",
            "yet-another-pass-000",
        )
        .await;
    assert!(reuse_err.is_err());

    Ok(())
}

#[tokio::test]
async fn test_database_token_store_prune_expired() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let store = ash_authentication::DatabaseTokenStore::new(ctx.clone());

    let user_id = Uuid::new_v4();
    // Token that expired in the past
    store
        .store_token("expired-jti-1", user_id, "refresh", 100, None)
        .await?;
    // Token valid until year 2100
    store
        .store_token("valid-jti-1", user_id, "refresh", 4102444800, None)
        .await?;

    // Expired token is not returned as valid
    assert!(
        store
            .get_valid_token("expired-jti-1", "refresh")
            .await?
            .is_none()
    );
    // Valid token is returned
    assert!(
        store
            .get_valid_token("valid-jti-1", "refresh")
            .await?
            .is_some()
    );

    // Prune expired
    let pruned = store.prune_expired().await?;
    assert_eq!(pruned, 1);

    // Valid token remains
    assert!(
        store
            .get_valid_token("valid-jti-1", "refresh")
            .await?
            .is_some()
    );

    Ok(())
}

#[tokio::test]
async fn test_sqlite_database_token_store_and_rotation() -> Result<()> {
    let db = ash_sqlite::Sqlite::memory().await?;
    db.install(&[&User::DEF, &ash_authentication::AshToken::DEF])
        .await?;

    let ctx = Context::new(db);
    let jwt_service = JwtService::new("test-super-secret-sqlite-jwt-key-1234567890");
    let strategy = User::auth_strategy().with_jwt_service(jwt_service);

    let _user: User = User::register_with_password(&ctx)
        .email("sqlite_user@example.com")
        .password("sqlite-pass-123")
        .password_confirmation("sqlite-pass-123")
        .await
        .expect("registration in sqlite should succeed");

    let (_, pair1) = strategy
        .sign_in_with_password_and_token_pair(
            &ctx,
            "sqlite_user@example.com",
            "sqlite-pass-123",
        )
        .await
        .expect("sqlite sign-in with token pair should succeed");

    // Rotate in SQLite
    let (_, pair2) = strategy
        .rotate_refresh_token(&ctx, &pair1.refresh_token)
        .await
        .expect("sqlite refresh rotation should succeed");

    assert_ne!(pair1.access_token, pair2.access_token);
    assert_ne!(pair1.refresh_token, pair2.refresh_token);

    // Replay attack in SQLite fails
    let replay_err = strategy
        .rotate_refresh_token(&ctx, &pair1.refresh_token)
        .await;
    assert!(replay_err.is_err());

    Ok(())
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn test_axum_full_auth_lifecycle() -> Result<()> {
    use axum::http::StatusCode;
    use axum::Router;
    use serde_json::json;
    use tower::ServiceExt;

    let ctx = Context::new(Memory::new());
    let jwt_service = Arc::new(JwtService::new(
        "test-super-secret-lifecycle-jwt-key-1234567890",
    ));

    // Register user
    let user: User = User::register_with_password(&ctx)
        .email("lifecycle_user@example.com")
        .password("initial-pass-123")
        .password_confirmation("initial-pass-123")
        .await
        .expect("user registration should succeed");

    let strategy = User::auth_strategy();

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

    let auth_routes = auth_router(strategy, jwt_service.clone(), ctx);
    let app: Router = Router::new().nest("/auth", auth_routes).with_state(app_state);

    // 1. POST /auth/sign-in
    let sign_in_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/sign-in")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "identity": "lifecycle_user@example.com",
                    "password": "initial-pass-123"
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
    let initial_access_token = auth_res.token;
    let initial_refresh_token = auth_res.refresh_token.expect("refresh token should be present");

    // 2. POST /auth/refresh
    let refresh_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/refresh")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "refresh_token": initial_refresh_token
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(refresh_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let refresh_res: ash_authentication::AuthTokenResponse =
        serde_json::from_slice(&body_bytes).unwrap();
    assert_ne!(initial_access_token, refresh_res.token);
    let second_access_token = refresh_res.token;

    // 3. Replay attack on /auth/refresh with old refresh token must be rejected (401)
    let replay_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/refresh")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "refresh_token": initial_refresh_token
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(replay_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 4. POST /auth/change-password
    let change_pwd_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/change-password")
        .header("Authorization", format!("Bearer {second_access_token}"))
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "current_password": "initial-pass-123",
                    "new_password": "changed-pass-789",
                    "password_confirmation": "changed-pass-789"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(change_pwd_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 5. POST /auth/request-password-reset
    let reset_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/request-password-reset")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "email": "lifecycle_user@example.com"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(reset_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let reset_req_val: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let reset_token = reset_req_val["reset_token"].as_str().unwrap().to_string();

    // 6a. POST /auth/reset-password with password confirmation mismatch must fail (401)
    let mismatch_reset_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/reset-password")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "reset_token": reset_token,
                    "new_password": "brand-new-pass-321",
                    "password_confirmation": "mismatch-pass"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(mismatch_reset_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // 6b. POST /auth/reset-password with valid confirmation succeeds
    let reset_confirm_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/reset-password")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "reset_token": reset_token,
                    "new_password": "brand-new-pass-321",
                    "password_confirmation": "brand-new-pass-321"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(reset_confirm_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 7. Verify sign in with the reset password succeeds
    let sign_in_new_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/sign-in")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "identity": "lifecycle_user@example.com",
                    "password": "brand-new-pass-321"
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(sign_in_new_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let body_bytes = http_body_util::BodyExt::collect(res.into_body())
        .await
        .unwrap()
        .to_bytes();
    let sign_in_new_res: ash_authentication::AuthTokenResponse =
        serde_json::from_slice(&body_bytes).unwrap();
    let active_token = sign_in_new_res.token;

    // 8. POST /auth/revoke
    let revoke_req = axum::http::Request::builder()
        .method("POST")
        .uri("/auth/revoke")
        .header("Content-Type", "application/json")
        .body(http_body_util::BodyExt::boxed(
            http_body_util::Full::from(
                json!({
                    "token": active_token
                })
                .to_string(),
            ),
        ))
        .unwrap();

    let res = app.clone().oneshot(revoke_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 9. Subsequent GET /auth/me with revoked token returns 401 UNAUTHORIZED
    let me_req = axum::http::Request::builder()
        .method("GET")
        .uri("/auth/me")
        .header("Authorization", format!("Bearer {active_token}"))
        .body(http_body_util::BodyExt::boxed(http_body_util::Full::new(
            bytes::Bytes::new(),
        )))
        .unwrap();

    let res = app.oneshot(me_req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    Ok(())
}
