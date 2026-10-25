//! Authentication on Postgres with schema-per-tenant tenancy: does the auth
//! strategy read users from the request's tenant schema?
//!
//! Needs a local Postgres. DATABASE_URL defaults to
//! postgres://postgres:postgres@localhost/ashrust_audit_auth.

use ash_authentication::{ApiKeyService, AshToken, JwtService, authentication};
use ash_core::Resource;
use ash_postgres::Postgres;
use uuid::Uuid;

const SECRET: &str = "audit-tenant-binding-pg-secret-key-0123456789";

#[authentication]
ash_core::resource! {
    User {
        table "audit_ctx_users";
        multitenancy { strategy: context; }
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
            strategy tokens { token_lifetime_secs: 3600; }
            strategy api_key { api_key_field: api_key_hash; key_prefix: "ash_"; }
        }
        actions {
            read read { primary; }
            update set_api_key { accept [api_key_hash]; }
        }
    }
}

async fn db() -> (Postgres, String) {
    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/ashrust_audit_auth".into());
    let admin = Postgres::connect(&url).await.expect("local postgres");
    let schema = format!("auth_schema_{}", Uuid::new_v4().simple());
    admin
        .execute_sql(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    let separator = if url.contains('?') { '&' } else { '?' };
    let db = Postgres::connect(&format!(
        "{url}{separator}options=-c%20search_path%3D{schema}%2Cpublic"
    ))
    .await
    .unwrap();
    db.install(&[&User::DEF, &AshToken::DEF]).await.unwrap();
    (db, schema)
}

#[tokio::test]
async fn credential_lookups_use_the_tenant_schema() {
    let (db, storage_schema) = db().await;
    let tenant = format!("audit_t_{}", Uuid::new_v4().simple());
    let other_tenant = format!("audit_t_{}", Uuid::new_v4().simple());
    db.install_tenant(&tenant, &[&User::DEF]).await.unwrap();
    db.install_tenant(&other_tenant, &[&User::DEF])
        .await
        .unwrap();
    let root_runtime = ash_core::Runtime::new(db.clone());
    let root = root_runtime.bootstrap();
    let a = root_runtime.bind(root.actor().cloned(), Some(tenant.clone()));

    let alice = User::register_with_password(&a)
        .email("alice@a.example")
        .password("alice-password-1")
        .password_confirmation("alice-password-1")
        .await
        .unwrap();
    // Control: the normal read path finds alice in her tenant schema.
    let found = User::get(&a, alice.id).await;
    assert!(
        found.is_ok(),
        "control: User::get in tenant {tenant}: {found:?}"
    );

    let strategy = User::auth_strategy(&root_runtime).with_jwt_service(JwtService::new(SECRET));
    let signed_in = strategy
        .sign_in_with_password(&a, "alice@a.example", "alice-password-1")
        .await;
    let by_id = strategy.get_user_by_id(&a, alice.id).await;
    assert!(
        signed_in.is_ok() && by_id.is_ok(),
        "strategy reads ignore the tenant schema: sign_in={:?} get_user_by_id={:?}",
        signed_in.map(|u| u.id),
        by_id.map(|u| u.id)
    );
    let b = root_runtime.bind(root.actor().cloned(), Some((&other_tenant).into()));
    assert!(
        strategy
            .sign_in_with_password(&b, "alice@a.example", "alice-password-1")
            .await
            .is_err()
    );
    assert!(strategy.get_user_by_id(&b, alice.id).await.is_err());
    assert!(strategy.get_user_by_id(&root, alice.id).await.is_err());

    let (raw, hash) = ApiKeyService::default().generate_api_key();
    alice
        .clone()
        .set_api_key_on(&a)
        .api_key_hash(Some(hash))
        .await
        .unwrap();
    assert_eq!(
        strategy.authenticate_api_key(&a, &raw).await.unwrap().id,
        alice.id
    );
    assert!(strategy.authenticate_api_key(&b, &raw).await.is_err());
    assert!(
        strategy
            .request_password_reset(&b, "alice@a.example")
            .await
            .is_err()
    );
    let (_, reset) = strategy
        .request_password_reset(&a, "alice@a.example")
        .await
        .unwrap();
    assert!(
        strategy
            .reset_password_with_token(&b, &reset, "new-password-1", "new-password-1")
            .await
            .is_err()
    );
    assert!(
        strategy
            .reset_password_with_token(&a, &reset, "new-password-1", "new-password-1")
            .await
            .is_ok()
    );
    db.execute_sql(&format!(
        "DROP SCHEMA {tenant} CASCADE; DROP SCHEMA {other_tenant} CASCADE; DROP SCHEMA {storage_schema} CASCADE"
    ))
    .await
    .unwrap();
}
