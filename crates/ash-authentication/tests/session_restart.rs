use ash_authentication::{AshToken, JwtService, authentication};
use ash_core::{Resource, Runtime};
use uuid::Uuid;

const SECRET: &str = "restart-tests-signing-key-0123456789";
const PASSWORD: &str = "member-password-123";

#[authentication]
ash_core::resource! {
    User {
        table "restart_users";
        attributes {
            id: Uuid [pk];
            email: String;
            role: String = "member";
            credential_version: i64 = 0;
        }
        authentication { strategy password { identity_field: email; } strategy tokens {} }
        actions {
            read read { primary; }
            update change_role { accept [role]; }
            destroy destroy { primary; }
        }
    }
}

#[tokio::test]
async fn sqlite_revocation_survives_closing_and_reopening_the_database() {
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/step10/restarts");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{}.db", Uuid::new_v4().simple()));
    let (revoked, valid) = {
        let data = ash_sqlite::Sqlite::file(&path).await.unwrap();
        data.install(&[&User::DEF, &AshToken::DEF]).await.unwrap();
        let runtime = Runtime::new(data);
        let ctx = runtime.bootstrap();
        User::register_with_password(&ctx)
            .email("member@example.test")
            .password(PASSWORD)
            .password_confirmation(PASSWORD)
            .await
            .unwrap();
        let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
        let (_, revoked) = strategy
            .sign_in_with_password_and_token(&ctx, "member@example.test", PASSWORD)
            .await
            .unwrap();
        let (_, valid) = strategy
            .sign_in_with_password_and_token(&ctx, "member@example.test", PASSWORD)
            .await
            .unwrap();
        let other = Runtime::new(ash_sqlite::Sqlite::file(&path).await.unwrap());
        let other_strategy = User::auth_strategy(&other).with_jwt_service(JwtService::new(SECRET));
        assert!(
            other_strategy
                .authenticate(&other.bootstrap(), &revoked)
                .await
                .is_ok()
        );
        strategy.revoke_token(&ctx, &revoked).await.unwrap();
        assert!(
            other_strategy
                .authenticate(&other.bootstrap(), &revoked)
                .await
                .is_err()
        );
        assert!(
            other_strategy
                .authenticate(&other.bootstrap(), &valid)
                .await
                .is_ok()
        );
        (revoked, valid)
    };
    let runtime = Runtime::new(ash_sqlite::Sqlite::file(&path).await.unwrap());
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    assert!(
        strategy
            .authenticate(&runtime.bootstrap(), &revoked)
            .await
            .is_err()
    );
    assert!(
        strategy
            .authenticate(&runtime.bootstrap(), &valid)
            .await
            .is_ok()
    );
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn extractor_reads_live_actor_state_and_rejects_revoked_or_deleted_sessions() {
    use axum::{Json, Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;
    let runtime = Runtime::new(ash_memory::Memory::new());
    let ctx = runtime.bootstrap();
    let user = User::register_with_password(&ctx)
        .email("member@example.test")
        .password(PASSWORD)
        .password_confirmation(PASSWORD)
        .await
        .unwrap();
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    let (_, revoked) = strategy
        .sign_in_with_password_and_token(&ctx, "member@example.test", PASSWORD)
        .await
        .unwrap();
    let (_, valid) = strategy
        .sign_in_with_password_and_token(&ctx, "member@example.test", PASSWORD)
        .await
        .unwrap();
    async fn identity(
        ash_authentication::AuthUser(identity): ash_authentication::AuthUser,
    ) -> Json<serde_json::Value> {
        Json(serde_json::json!({"role": identity.actor().role(), "tenant": identity.tenant()}))
    }
    let app: Router = Router::new().route("/identity", get(identity)).with_state(
        ash_authentication::RequestAuthenticator::new(strategy.clone(), ctx.clone()),
    );
    let request = |token: &str| {
        Request::builder()
            .uri("/identity")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request(&valid)).await.unwrap().status(),
        200
    );
    user.change_role_on(&ctx).role("admin").await.unwrap();
    let response = app.clone().oneshot(request(&valid)).await.unwrap();
    assert_eq!(response.status(), 200);
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .unwrap()
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["role"], "admin");
    assert!(body["tenant"].is_null());
    strategy.revoke_token(&ctx, &revoked).await.unwrap();
    assert_eq!(
        app.clone()
            .oneshot(request(&revoked))
            .await
            .unwrap()
            .status(),
        401
    );
    assert_eq!(
        app.clone().oneshot(request(&valid)).await.unwrap().status(),
        200
    );
    user.destroy_on(&ctx).await.unwrap();
    assert_eq!(app.oneshot(request(&valid)).await.unwrap().status(), 401);
}
