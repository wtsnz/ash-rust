//! Authentication with attribute tenancy: credential lookups and signed tenant claims.
//! Each scenario runs on memory, SQLite and Postgres with a same-tenant control.

use ash_authentication::{
    ApiKeyService, AshToken, AuthStrategy, DatabaseTokenStore, JwtService, authentication,
};
use ash_core::{Context, DataLayer, Resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

const SECRET: &str = "audit-tenant-binding-secret-key-0123456789";
const EMAIL: &str = "alice@a.example";
const PASSWORD: &str = "alice-password-1";
const NEW_PASSWORD: &str = "alice-password-2";

#[authentication]
ash_core::resource! {
    User {
        table "tenant_binding_users";
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            email: String;
        }
        authentication {
            strategy password {
                identity_field: email;
                hashed_password_field: hashed_password;
                min_password_length: 8;
                require_confirmation: true;
            }
            strategy tokens { token_lifetime_secs: 3600; }
            strategy api_key { api_key_field: api_key_hash; key_prefix: "ash_"; }
        }
        actions {
            read read { primary; }
            update set_api_key { accept [api_key_hash]; }
        }
    }
}

struct Fixture<D: DataLayer> {
    root: Context<D>,
    a: Context<D>,
    b: Context<D>,
    strategy: AuthStrategy<User>,
    alice: User,
}

async fn fixture<D: DataLayer>(data: D) -> Fixture<D> {
    let root_runtime = ash_core::Runtime::new(data);
    let root = root_runtime.bootstrap();
    let a = root_runtime.bind(root.actor().cloned(), Some(("a").into()));
    let b = root_runtime.bind(root.actor().cloned(), Some(("b").into()));
    let alice = User::register_with_password(&a)
        .email(EMAIL)
        .password(PASSWORD)
        .password_confirmation(PASSWORD)
        .await
        .unwrap();
    let strategy = User::auth_strategy(&root_runtime).with_jwt_service(JwtService::new(SECRET));
    Fixture {
        root,
        a,
        b,
        strategy,
        alice,
    }
}

macro_rules! on_backends {
    ($scenario:ident) => {
        mod $scenario {
            #[tokio::test]
            async fn memory() {
                super::$scenario(super::fixture(super::Memory::new()).await).await;
            }
            #[tokio::test]
            async fn sqlite() {
                let db = super::Sqlite::memory().await.unwrap();
                db.install(&[
                    &<super::User as super::Resource>::DEF,
                    &<super::AshToken as super::Resource>::DEF,
                ])
                .await
                .unwrap();
                super::$scenario(super::fixture(db).await).await;
            }
            #[tokio::test]
            async fn postgres() {
                let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
                    "postgres://postgres:postgres@localhost/ashrust_audit_auth".into()
                });
                let admin = ash_postgres::Postgres::connect(&url).await.unwrap();
                let schema = format!("auth_binding_{}", uuid::Uuid::new_v4().simple());
                admin
                    .execute_sql(&format!("CREATE SCHEMA {schema}"))
                    .await
                    .unwrap();
                let separator = if url.contains('?') { '&' } else { '?' };
                let db = ash_postgres::Postgres::connect(&format!(
                    "{url}{separator}options=-c%20search_path%3D{schema}%2Cpublic"
                ))
                .await
                .unwrap();
                db.install(&[
                    &<super::User as super::Resource>::DEF,
                    &<super::AshToken as super::Resource>::DEF,
                ])
                .await
                .unwrap();
                super::$scenario(super::fixture(db).await).await;
                admin
                    .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
                    .await
                    .unwrap();
            }
        }
    };
}

async fn sign_in_is_scoped_to_request_tenant<D: DataLayer>(f: Fixture<D>) {
    let (alice, pair) = f
        .strategy
        .sign_in_with_password_and_token_pair(&f.a, EMAIL, PASSWORD)
        .await
        .unwrap();
    assert_eq!(alice.id, f.alice.id);
    let claims = f
        .strategy
        .jwt_service()
        .unwrap()
        .verify_token(&pair.access_token)
        .unwrap();
    assert_eq!(claims.tenant.as_deref(), Some("a"));
    assert!(
        f.strategy
            .sign_in_with_password(&f.b, EMAIL, PASSWORD)
            .await
            .is_err()
    );

    // Identity values may repeat across tenants without making sign-in ambiguous.
    let other = User::register_with_password(&f.b)
        .email(EMAIL)
        .password(NEW_PASSWORD)
        .password_confirmation(NEW_PASSWORD)
        .await
        .unwrap();
    assert_eq!(
        f.strategy
            .sign_in_with_password(&f.b, EMAIL, NEW_PASSWORD)
            .await
            .unwrap()
            .id,
        other.id
    );
    assert!(
        f.strategy
            .sign_in_with_password(&f.b, EMAIL, PASSWORD)
            .await
            .is_err()
    );
}
on_backends!(sign_in_is_scoped_to_request_tenant);

async fn subject_lookup_is_scoped_to_request_tenant<D: DataLayer>(f: Fixture<D>) {
    assert_eq!(
        f.strategy
            .get_user_by_id(&f.a, f.alice.id)
            .await
            .unwrap()
            .id,
        f.alice.id
    );
    assert!(f.strategy.get_user_by_id(&f.b, f.alice.id).await.is_err());
}
on_backends!(subject_lookup_is_scoped_to_request_tenant);

async fn refresh_token_is_bound_to_its_tenant<D: DataLayer>(f: Fixture<D>) {
    let (_, pair) = f
        .strategy
        .sign_in_with_password_and_token_pair(&f.a, EMAIL, PASSWORD)
        .await
        .unwrap();
    assert!(
        f.strategy
            .rotate_refresh_token(&f.b, &pair.refresh_token)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .rotate_refresh_token(&f.root, &pair.refresh_token)
            .await
            .is_err()
    );
    // Rejection must happen before consumption: the original credential still works.
    let (user, rotated) = f
        .strategy
        .rotate_refresh_token(&f.a, &pair.refresh_token)
        .await
        .unwrap();
    assert_eq!(user.id, f.alice.id);
    let claims = f
        .strategy
        .jwt_service()
        .unwrap()
        .verify_token(&rotated.access_token)
        .unwrap();
    assert_eq!(claims.tenant.as_deref(), Some("a"));
}
on_backends!(refresh_token_is_bound_to_its_tenant);

async fn api_key_lookup_is_scoped_to_tenant<D: DataLayer>(f: Fixture<D>) {
    let (raw, hash) = ApiKeyService::default().generate_api_key();
    f.alice
        .clone()
        .set_api_key_on(&f.a)
        .api_key_hash(Some(hash))
        .await
        .unwrap();
    assert_eq!(
        f.strategy
            .authenticate_api_key(&f.a, &raw)
            .await
            .unwrap()
            .id,
        f.alice.id
    );
    assert!(f.strategy.authenticate_api_key(&f.b, &raw).await.is_err());
}
on_backends!(api_key_lookup_is_scoped_to_tenant);

async fn request_password_reset_is_scoped_to_tenant<D: DataLayer>(f: Fixture<D>) {
    assert!(
        f.strategy
            .request_password_reset(&f.b, EMAIL)
            .await
            .is_err()
    );
    let (user, token) = f
        .strategy
        .request_password_reset(&f.a, EMAIL)
        .await
        .unwrap();
    assert_eq!(user.id, f.alice.id);
    let claims = f
        .strategy
        .jwt_service()
        .unwrap()
        .verify_token_with_purpose(&token, "password_reset")
        .unwrap();
    assert_eq!(claims.tenant.as_deref(), Some("a"));
}
on_backends!(request_password_reset_is_scoped_to_tenant);

async fn reset_token_tenant_claim_is_checked<D: DataLayer>(f: Fixture<D>) {
    let (_, token) = f
        .strategy
        .request_password_reset(&f.a, EMAIL)
        .await
        .unwrap();
    assert!(
        f.strategy
            .reset_password_with_token(&f.b, &token, NEW_PASSWORD, NEW_PASSWORD)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .reset_password_with_token(&f.root, &token, NEW_PASSWORD, NEW_PASSWORD)
            .await
            .is_err()
    );
    assert!(
        f.strategy
            .sign_in_with_password(&f.a, EMAIL, PASSWORD)
            .await
            .is_ok()
    );
    // A rejected request neither changes the password nor consumes the token.
    let user = f
        .strategy
        .reset_password_with_token(&f.a, &token, NEW_PASSWORD, NEW_PASSWORD)
        .await
        .unwrap();
    assert_eq!(user.id, f.alice.id);
    assert!(
        f.strategy
            .sign_in_with_password(&f.a, EMAIL, NEW_PASSWORD)
            .await
            .is_ok()
    );
}
on_backends!(reset_token_tenant_claim_is_checked);

async fn inconsistent_or_tenantless_claims_cannot_reset_a_password<D: DataLayer>(f: Fixture<D>) {
    let jwt = f.strategy.jwt_service().unwrap();
    // Seed credentials directly: the fixed lookup cannot issue these inconsistent
    // tokens. Cover the subject-in-A, claim-in-B case too.
    for tenant in [Some("b".to_string()), None] {
        let token = jwt.sign_reset_token(f.alice.id, tenant).unwrap();
        let claims = jwt
            .verify_token_with_purpose(&token, "password_reset")
            .unwrap();
        let store = DatabaseTokenStore::new(f.a.clone());
        store
            .store_token(
                &claims.jti,
                f.alice.id,
                "password_reset",
                claims.exp as i64,
                None,
            )
            .await
            .unwrap();
        assert!(
            f.strategy
                .reset_password_with_token(&f.b, &token, NEW_PASSWORD, NEW_PASSWORD)
                .await
                .is_err()
        );
        assert!(
            f.strategy
                .reset_password_with_token(&f.a, &token, NEW_PASSWORD, NEW_PASSWORD)
                .await
                .is_err()
        );
        assert!(
            store
                .get_valid_token(&claims.jti, "password_reset")
                .await
                .unwrap()
                .is_some()
        );
    }
    assert!(
        f.strategy
            .sign_in_with_password(&f.a, EMAIL, PASSWORD)
            .await
            .is_ok()
    );
}
on_backends!(inconsistent_or_tenantless_claims_cannot_reset_a_password);

async fn credential_lookups_require_a_tenant<D: DataLayer>(f: Fixture<D>) {
    let (raw, hash) = ApiKeyService::default().generate_api_key();
    f.alice
        .clone()
        .set_api_key_on(&f.a)
        .api_key_hash(Some(hash))
        .await
        .unwrap();
    let errors = [
        f.strategy
            .sign_in_with_password(&f.root, EMAIL, PASSWORD)
            .await
            .unwrap_err(),
        f.strategy
            .get_user_by_id(&f.root, f.alice.id)
            .await
            .unwrap_err(),
        f.strategy
            .request_password_reset(&f.root, EMAIL)
            .await
            .unwrap_err(),
        f.strategy
            .authenticate_api_key(&f.root, &raw)
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert!(
            matches!(
                error,
                ash_authentication::AuthError::Core(ash_core::Error::TenantRequired { .. })
            ),
            "{error}"
        );
    }
}
on_backends!(credential_lookups_require_a_tenant);

#[cfg(feature = "axum")]
#[tokio::test]
async fn auth_user_extractor_preserves_signed_tenant() {
    use ash_authentication::AuthUser;
    use axum::{Json, Router, routing::get};
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn whoami(AuthUser(actor): AuthUser) -> Json<serde_json::Value> {
        Json(
            serde_json::json!({ "tenant": actor.attr("tenant").map(ash_core::Value::to_plain_json) }),
        )
    }
    let jwt = Arc::new(JwtService::new(SECRET));
    let app: Router = Router::new()
        .route("/whoami", get(whoami))
        .with_state(jwt.clone());
    for tenant in [Some("a".to_string()), None] {
        let token = jwt
            .sign_access_token(Uuid::new_v4(), None, tenant.clone())
            .unwrap();
        let req = axum::http::Request::builder()
            .uri("/whoami")
            .header("Authorization", format!("Bearer {token}"))
            .header("x-tenant", "b")
            .body(axum::body::Body::empty())
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        assert_eq!(res.status(), 200);
        let body = http_body_util::BodyExt::collect(res.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["tenant"], serde_json::json!(tenant));
    }
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn auth_router_checks_access_token_tenant_before_me_or_password_change() {
    use axum::{
        Router,
        body::Body,
        http::{Request, StatusCode},
    };
    use serde_json::json;
    use std::sync::Arc;
    use tower::ServiceExt;

    let f = fixture(Memory::new()).await;
    let jwt = Arc::new(JwtService::new(SECRET));
    let app: Router = ash_authentication::auth_router(f.strategy.clone(), jwt.clone(), f.a.clone());
    for tenant in [Some("b".to_string()), None] {
        let token = jwt.sign_access_token(f.alice.id, None, tenant).unwrap();
        for (method, path, body) in [
            ("GET", "/me", json!(null)),
            (
                "POST",
                "/change-password",
                json!({ "current_password": PASSWORD, "new_password": NEW_PASSWORD, "password_confirmation": NEW_PASSWORD }),
            ),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(path)
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            assert_eq!(
                app.clone().oneshot(request).await.unwrap().status(),
                StatusCode::UNAUTHORIZED
            );
        }
    }
    assert!(
        f.strategy
            .sign_in_with_password(&f.a, EMAIL, PASSWORD)
            .await
            .is_ok()
    );
    let token = jwt
        .sign_access_token(f.alice.id, None, Some("a".into()))
        .unwrap();
    let request = Request::builder()
        .uri("/me")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(request).await.unwrap().status(), StatusCode::OK);
}
