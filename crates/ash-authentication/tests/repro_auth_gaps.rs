//! Auth tests that encode the behavior we want. They fail on the current crate.

use ash_authentication::{AuthError, authentication};
use ash_core::{Context, Result};
use ash_memory::Memory;
use uuid::Uuid;

#[authentication]
ash_core::resource! {
User {
    table "repro_auth_users";

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

    strategy confirmation {
        confirmed_field: confirmed_at;
        prevent_unconfirmed_sign_in: true;
    }
}

actions {
    read read { primary; }
}
}}

#[tokio::test]
async fn unconfirmed_user_cannot_sign_in() -> Result<()> {
    let ctx = Context::new(Memory::new());

    User::register_with_password(&ctx)
        .email("unconfirmed@example.com")
        .password("strong-password-123")
        .password_confirmation("strong-password-123")
        .await
        .expect("register");

    let err = User::auth_strategy()
        .sign_in_with_password(&ctx, "unconfirmed@example.com", "strong-password-123")
        .await
        .expect_err("prevent_unconfirmed_sign_in is true and confirmed_at is unset");

    assert!(
        matches!(err, AuthError::UserUnconfirmed),
        "sign_in_with_password never reads ConfirmationStrategyDef; got {err}"
    );
    Ok(())
}

#[cfg(feature = "axum")]
mod http {
    use std::sync::Arc;

    use ash_authentication::{JwtService, auth_router};
    use ash_core::{Context, DataLayer, Resource, Result, Value};
    use ash_memory::Memory;
    use axum::Router;
    use axum::http::StatusCode;
    use serde_json::json;
    use tower::ServiceExt;

    use super::User;

    #[derive(Clone)]
    struct AppState {
        jwt: Arc<JwtService>,
    }

    impl axum::extract::FromRef<AppState> for Arc<JwtService> {
        fn from_ref(state: &AppState) -> Self {
            state.jwt.clone()
        }
    }

    async fn app() -> (Router, Context<Memory>, Arc<JwtService>) {
        let ctx = Context::new(Memory::new());
        let jwt = Arc::new(JwtService::new(
            "repro-auth-gaps-secret-key-at-least-32-bytes",
        ));
        let user = User::register_with_password(&ctx)
            .email("will@example.com")
            .password("correct-horse-battery")
            .password_confirmation("correct-horse-battery")
            .await
            .unwrap();
        let mut fields = user.to_fields();
        fields.insert(
            "confirmed_at".into(),
            Value::String("2026-01-01T00:00:00Z".into()),
        );
        ctx.data
            .update(&User::DEF, user.id, fields)
            .await
            .expect("mark fixture user confirmed so HTTP tests can sign in");

        let strategy = User::auth_strategy();
        let routes = auth_router(strategy, jwt.clone(), ctx.clone());
        let router = Router::new()
            .nest("/auth", routes)
            .with_state(AppState { jwt: jwt.clone() });
        (router, ctx, jwt)
    }

    async fn json_body(res: axum::http::Response<axum::body::Body>) -> serde_json::Value {
        let bytes = http_body_util::BodyExt::collect(res.into_body())
            .await
            .unwrap()
            .to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn request_password_reset_does_not_return_the_token() -> Result<()> {
        let (app, _, _) = app().await;

        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/auth/request-password-reset")
            .header("Content-Type", "application/json")
            .body(http_body_util::BodyExt::boxed(http_body_util::Full::from(
                json!({ "email": "will@example.com" }).to_string(),
            )))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK);
        let body = json_body(res).await;
        assert!(
            body.get("reset_token").is_none(),
            "reset JWT is in the HTTP body: {body}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn refresh_token_is_rejected_as_bearer() -> Result<()> {
        let (app, ctx, jwt) = app().await;
        let (_user, pair) = User::auth_strategy()
            .with_jwt_service((*jwt).clone())
            .sign_in_with_password_and_token_pair(&ctx, "will@example.com", "correct-horse-battery")
            .await
            .unwrap();

        let req = axum::http::Request::builder()
            .method("GET")
            .uri("/auth/me")
            .header("Authorization", format!("Bearer {}", pair.refresh_token))
            .body(http_body_util::BodyExt::boxed(http_body_util::Full::from(
                Vec::<u8>::new(),
            )))
            .unwrap();

        let res = app.oneshot(req).await.unwrap();
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "/me used verify_token, not verify_token_with_purpose(\"access\")"
        );
        Ok(())
    }
}
