use ash_authentication::{AshToken, AuthStrategy, JwtService, authentication};
use ash_core::{Context, DataLayer, Resource, Runtime};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

const SECRET: &str = "local-auth-behavior-secret-key-0123456789";
const PASSWORD: &str = "strong-password-123";
const EMAIL: &str = "member@example.test";

#[authentication]
ash_core::resource! {
    User {
        table "local_behavior_users";
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            email: String;
        }
        authentication {
            strategy password { identity_field: email; }
            strategy tokens { token_lifetime_secs: 3600; }
            strategy api_key { api_key_field: api_key_hash; key_prefix: "service_live_"; }
        }
        actions {
            read read { primary; }
            update set_key { accept [api_key_hash]; }
        }
    }
}

struct Fixture<D: DataLayer> {
    runtime: Runtime<D>,
    ctx: Context<D>,
    strategy: AuthStrategy<User>,
    user: User,
}

async fn fixture<D: DataLayer>(data: D) -> Fixture<D> {
    let runtime = Runtime::new(data);
    let ctx = runtime.bind(None, Some("one".into()));
    let strategy = User::auth_strategy(&runtime).with_jwt_service(JwtService::new(SECRET));
    let user = User::register_with_password(&ctx)
        .email(EMAIL)
        .password(PASSWORD)
        .password_confirmation(PASSWORD)
        .await
        .unwrap();
    Fixture {
        runtime,
        ctx,
        strategy,
        user,
    }
}

macro_rules! on_backends {
    ($scenario:ident) => {
        mod $scenario {
            use ash_core::Resource;
            #[tokio::test]
            async fn memory() {
                super::$scenario(super::fixture(super::Memory::new()).await).await;
            }
            #[tokio::test]
            async fn sqlite() {
                let data = super::Sqlite::memory().await.unwrap();
                data.install(&[&super::User::DEF, &super::AshToken::DEF])
                    .await
                    .unwrap();
                super::$scenario(super::fixture(data).await).await;
            }
            #[tokio::test]
            async fn postgres() {
                let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
                let admin = ash_postgres::Postgres::connect(&url).await.unwrap();
                let schema = format!("local_auth_{}", uuid::Uuid::new_v4().simple());
                admin
                    .execute_sql(&format!("CREATE SCHEMA {schema}"))
                    .await
                    .unwrap();
                let separator = if url.contains('?') { '&' } else { '?' };
                let data = ash_postgres::Postgres::connect(&format!(
                    "{url}{separator}options=-c%20search_path%3D{schema}%2Cpublic"
                ))
                .await
                .unwrap();
                data.install(&[&super::User::DEF, &super::AshToken::DEF])
                    .await
                    .unwrap();
                super::$scenario(super::fixture(data).await).await;
                admin
                    .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
                    .await
                    .unwrap();
            }
        }
    };
}

async fn session<D: DataLayer>(f: &Fixture<D>, email: &str) -> ash_authentication::TokenPair {
    f.strategy
        .sign_in_with_password_and_token_pair(&f.ctx, email, PASSWORD)
        .await
        .unwrap()
        .1
}

async fn token_rows<D: DataLayer>(f: &Fixture<D>) -> Vec<AshToken> {
    AshToken::query(&f.ctx).all().await.unwrap()
}

async fn configured_prefix_generates_and_authenticates_keys<D: DataLayer>(f: Fixture<D>) {
    let service = f.strategy.api_key_service();
    let (key, hash) = service.generate_api_key();
    assert!(key.starts_with("service_live_"));
    f.user
        .set_key_on(&f.ctx)
        .api_key_hash(Some(hash))
        .await
        .unwrap();
    assert_eq!(
        f.strategy
            .authenticate_api_key(&f.ctx, &key)
            .await
            .unwrap()
            .id,
        f.user.id
    );
    let other_tenant = f.runtime.bind(None, Some("two".into()));
    assert!(
        f.strategy
            .authenticate_api_key(&other_tenant, &key)
            .await
            .is_err()
    );

    // Store the wrong-prefix key's actual hash, so rejection proves parsing,
    // rather than merely a missing hash in storage.
    let (wrong_key, wrong_hash) = ash_authentication::ApiKeyService::default().generate_api_key();
    let live = f.strategy.get_user_by_id(&f.ctx, f.user.id).await.unwrap();
    live.set_key_on(&f.ctx)
        .api_key_hash(Some(wrong_hash.clone()))
        .await
        .unwrap();
    assert!(
        f.strategy
            .authenticate_api_key(&f.ctx, &wrong_key)
            .await
            .is_err()
    );
    assert!(service.verify_api_key(&wrong_key, &wrong_hash).is_err());
    for malformed in [
        "service_live_short".to_string(),
        format!("service_live_{}", "z".repeat(64)),
    ] {
        let live = f.strategy.get_user_by_id(&f.ctx, f.user.id).await.unwrap();
        live.set_key_on(&f.ctx)
            .api_key_hash(Some(service.hash_api_key(&malformed)))
            .await
            .unwrap();
        assert!(
            f.strategy
                .authenticate_api_key(&f.ctx, &malformed)
                .await
                .is_err()
        );
    }
}
on_backends!(configured_prefix_generates_and_authenticates_keys);

async fn unknown_jti_revocation_creates_no_tombstones<D: DataLayer>(f: Fixture<D>) {
    let trusted = User::auth_strategy(&f.runtime);
    for _ in 0..3 {
        trusted
            .revoke_jti(&f.ctx, &Uuid::new_v4().to_string())
            .await
            .unwrap();
    }
    assert!(token_rows(&f).await.is_empty());
    let pair = session(&f, EMAIL).await;
    let jti = JwtService::new(SECRET)
        .verify_token(&pair.access_token)
        .unwrap()
        .jti;
    trusted.revoke_jti(&f.ctx, &jti).await.unwrap();
    assert_eq!(token_rows(&f).await.len(), 2);
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_err()
    );
}
on_backends!(unknown_jti_revocation_creates_no_tombstones);

async fn seed_token<D: DataLayer>(
    f: &Fixture<D>,
    resource: &str,
    tenant: &str,
    expires_at: i64,
    revoked: bool,
    consumed: bool,
) {
    AshToken::store(&f.ctx)
        .resource(resource)
        .tenant_key(serde_json::to_string(&Some(tenant)).unwrap())
        .jti(Uuid::new_v4().to_string())
        .subject(f.user.id)
        .purpose("access")
        .expires_at(expires_at)
        .revoked(revoked)
        .consumed(consumed)
        .extra(None)
        .await
        .unwrap();
}

async fn expired_revocations_are_pruned_with_namespace_isolation<D: DataLayer>(f: Fixture<D>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    for (revoked, consumed) in [(true, false), (false, true), (false, false)] {
        seed_token(&f, User::DEF.name, "one", now, revoked, consumed).await;
    }
    seed_token(&f, User::DEF.name, "one", now + 3600, true, false).await;
    seed_token(&f, User::DEF.name, "two", now, true, false).await;
    seed_token(&f, "OtherUser", "one", now, true, false).await;
    AshToken::store(&f.ctx)
        .resource(User::DEF.name)
        .tenant_key(serde_json::to_string(&Some("one")).unwrap())
        .jti("legacy-permanent-revocation")
        .subject(Uuid::nil())
        .purpose("revocation")
        .expires_at(i64::MAX)
        .revoked(true)
        .consumed(false)
        .extra(None)
        .await
        .unwrap();
    let store = f.strategy.token_store(&f.ctx);
    assert_eq!(store.prune_expired().await.unwrap(), 4);
    assert_eq!(store.prune_expired().await.unwrap(), 0);
    assert_eq!(token_rows(&f).await.len(), 3);

    seed_token(&f, User::DEF.name, "one", now, true, false).await;
    let pair = session(&f, EMAIL).await;
    assert_eq!(
        token_rows(&f).await.len(),
        5,
        "issuance prunes expired state"
    );
    seed_token(&f, User::DEF.name, "one", now, true, false).await;
    f.strategy
        .revoke_token(&f.ctx, &pair.access_token)
        .await
        .unwrap();
    assert_eq!(
        token_rows(&f).await.len(),
        5,
        "revocation prunes expired state"
    );
    assert!(
        f.strategy
            .authenticate(&f.ctx, &pair.access_token)
            .await
            .is_err()
    );
}
on_backends!(expired_revocations_are_pruned_with_namespace_isolation);

#[cfg(feature = "axum")]
mod http {
    use super::*;
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    fn app<D: DataLayer + 'static>(f: &Fixture<D>, strategy: AuthStrategy<User>) -> Router {
        ash_authentication::auth_router(strategy, Arc::new(JwtService::new(SECRET)), f.ctx.clone())
    }

    async fn post(
        app: &Router,
        path: &str,
        body: Value,
        bearer: Option<&str>,
    ) -> (StatusCode, Vec<u8>) {
        let mut request = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        if let Some(token) = bearer {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        (
            response.status(),
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .to_vec(),
        )
    }

    async fn reset_requests_hide_account_existence_and_delivery_errors<D: DataLayer + 'static>(
        f: Fixture<D>,
    ) {
        let messages = Arc::new(Mutex::new(Vec::new()));
        let sent = messages.clone();
        let strategy =
            f.strategy.clone().with_sender(
                ash_authentication::CallbackSender::new(
                    move |_: &str,
                          fields: &ash_core::FieldMap,
                          token: &str|
                          -> ash_authentication::BoxFuture<
                        'static,
                        ash_authentication::Result<()>,
                    > {
                        sent.lock()
                            .unwrap()
                            .push((fields.clone(), token.to_owned()));
                        Box::pin(async { Ok(()) })
                    },
                ),
            );
        let app = app(&f, strategy);
        let unknown = post(
            &app,
            "/request-password-reset",
            json!({"email":"unknown@example.test"}),
            None,
        )
        .await;
        assert_eq!(
            unknown,
            (StatusCode::OK, br#"{"status":"reset_requested"}"#.to_vec())
        );
        assert!(token_rows(&f).await.is_empty());
        assert!(messages.lock().unwrap().is_empty());
        let known = post(
            &app,
            "/request-password-reset",
            json!({"email":EMAIL}),
            None,
        )
        .await;
        assert_eq!(known, unknown);
        let token = {
            let sent = messages.lock().unwrap();
            assert_eq!(sent.len(), 1);
            assert_eq!(
                sent[0].0.get("email").and_then(ash_core::Value::as_str),
                Some(EMAIL)
            );
            sent[0].1.clone()
        };
        let claims = JwtService::new(SECRET)
            .verify_token_with_purpose(&token, "password_reset")
            .unwrap();
        assert_eq!(token_rows(&f).await.len(), 1);
        assert!(
            f.strategy
                .token_store(&f.ctx)
                .get_valid_token(&claims)
                .await
                .unwrap()
                .is_some()
        );

        let failing = f
            .strategy
            .clone()
            .with_sender(ash_authentication::CallbackSender::new(
            |_: &str,
             _: &ash_core::FieldMap,
             _: &str|
             -> ash_authentication::BoxFuture<'static, ash_authentication::Result<()>> {
                Box::pin(async {
                    Err(ash_authentication::AuthError::Crypto(
                        "delivery unavailable".into(),
                    ))
                })
            },
        ));
        let failed_app = super::http::app(&f, failing);
        assert_eq!(
            post(
                &failed_app,
                "/request-password-reset",
                json!({"email":EMAIL}),
                None
            )
            .await,
            unknown
        );
        assert_eq!(token_rows(&f).await.len(), 2, "mail failure follows commit");
    }
    on_backends!(reset_requests_hide_account_existence_and_delivery_errors);

    async fn public_revoke_rejects_arbitrary_jtis_without_writes<D: DataLayer + 'static>(
        f: Fixture<D>,
    ) {
        let app = app(&f, f.strategy.clone());
        for token in ["arbitrary-jti", "invalid.jwt.token", ""] {
            assert_eq!(
                post(&app, "/revoke", json!({"token":token}), None).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert!(token_rows(&f).await.is_empty());
        let pair = session(&f, EMAIL).await;
        let jti = JwtService::new(SECRET)
            .verify_token(&pair.access_token)
            .unwrap()
            .jti;
        assert_eq!(
            post(&app, "/revoke", json!({"token":jti}), None).await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(token_rows(&f).await.len(), 2);
        assert!(token_rows(&f).await.iter().all(|row| !row.revoked));
        assert!(
            f.strategy
                .authenticate(&f.ctx, &pair.access_token)
                .await
                .is_ok()
        );
    }
    on_backends!(public_revoke_rejects_arbitrary_jtis_without_writes);

    async fn verified_tokens_revoke_only_themselves<D: DataLayer + 'static>(f: Fixture<D>) {
        let app = app(&f, f.strategy.clone());
        let pair = session(&f, EMAIL).await;
        let jwt = JwtService::new(SECRET);
        for token in [
            JwtService::new("another-signing-secret-0123456789")
                .sign_access_token(f.user.id, None, Some("one".into()), User::DEF.name, 0)
                .unwrap(),
            jwt.sign_access_token(f.user.id, None, Some("two".into()), User::DEF.name, 0)
                .unwrap(),
            jwt.sign_access_token(f.user.id, None, Some("one".into()), "OtherUser", 0)
                .unwrap(),
            jwt.clone()
                .with_lifetime(std::time::Duration::ZERO)
                .sign_access_token(f.user.id, None, Some("one".into()), User::DEF.name, 0)
                .unwrap(),
        ] {
            assert_eq!(
                post(&app, "/revoke", json!({"token":token}), None).await.0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert!(
            f.strategy
                .authenticate(&f.ctx, &pair.access_token)
                .await
                .is_ok()
        );
        for _ in 0..2 {
            assert_eq!(
                post(&app, "/revoke", json!({"token":pair.access_token}), None).await,
                (StatusCode::OK, br#"{"status":"revoked"}"#.to_vec())
            );
        }
        assert!(
            f.strategy
                .authenticate(&f.ctx, &pair.access_token)
                .await
                .is_err()
        );
        assert_eq!(
            token_rows(&f)
                .await
                .iter()
                .filter(|row| row.revoked)
                .count(),
            1
        );
        let (_, next) = f
            .strategy
            .rotate_refresh_token(&f.ctx, &pair.refresh_token)
            .await
            .unwrap();
        assert!(
            f.strategy
                .authenticate(&f.ctx, &next.access_token)
                .await
                .is_ok()
        );
        assert_eq!(
            post(&app, "/revoke", json!({"token":next.refresh_token}), None)
                .await
                .0,
            StatusCode::OK
        );
        assert!(
            f.strategy
                .rotate_refresh_token(&f.ctx, &next.refresh_token)
                .await
                .is_err()
        );
    }
    on_backends!(verified_tokens_revoke_only_themselves);

    async fn authenticated_owners_revoke_only_their_sessions<D: DataLayer + 'static>(
        f: Fixture<D>,
    ) {
        let app = app(&f, f.strategy.clone());
        let pair = session(&f, EMAIL).await;
        let another = session(&f, EMAIL).await;
        let other_email = "another@example.test";
        User::register_with_password(&f.ctx)
            .email(other_email)
            .password(PASSWORD)
            .password_confirmation(PASSWORD)
            .await
            .unwrap();
        let other = session(&f, other_email).await;
        let other_ctx = f.runtime.bind(None, Some("two".into()));
        User::register_with_password(&other_ctx)
            .email(EMAIL)
            .password(PASSWORD)
            .password_confirmation(PASSWORD)
            .await
            .unwrap();
        let (_, other_tenant) = f
            .strategy
            .sign_in_with_password_and_token_pair(&other_ctx, EMAIL, PASSWORD)
            .await
            .unwrap();

        for bearer in [
            None,
            Some("arbitrary-jti"),
            Some(pair.refresh_token.as_str()),
            Some(other_tenant.access_token.as_str()),
        ] {
            assert_eq!(
                post(&app, "/revoke", json!({"all_sessions":true}), bearer)
                    .await
                    .0,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            post(
                &app,
                "/revoke",
                json!({"all_sessions":true,"subject":f.user.id}),
                Some(&other.access_token)
            )
            .await
            .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert!(token_rows(&f).await.iter().all(|row| !row.revoked));
        assert_eq!(
            post(
                &app,
                "/revoke",
                json!({"all_sessions":true}),
                Some(&pair.access_token)
            )
            .await
            .0,
            StatusCode::OK
        );
        for revoked in [&pair, &another] {
            assert!(
                f.strategy
                    .authenticate(&f.ctx, &revoked.access_token)
                    .await
                    .is_err()
            );
            assert!(
                f.strategy
                    .rotate_refresh_token(&f.ctx, &revoked.refresh_token)
                    .await
                    .is_err()
            );
        }
        assert!(
            f.strategy
                .authenticate(&f.ctx, &other.access_token)
                .await
                .is_ok()
        );
        assert!(
            f.strategy
                .authenticate(&other_ctx, &other_tenant.access_token)
                .await
                .is_ok()
        );
        assert_eq!(
            post(
                &app,
                "/revoke",
                json!({"all_sessions":true}),
                Some(&pair.access_token)
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    on_backends!(authenticated_owners_revoke_only_their_sessions);
}
