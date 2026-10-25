use std::path::PathBuf;

use ash_authentication::{AshToken, Claims, DatabaseTokenStore};
use ash_core::{DataLayer, Resource, Runtime, TransactionSupport};
use uuid::Uuid;

ash_core::resource! {
    LegacyToken {
        table "ash_tokens";
        attributes {
            id: Uuid [pk];
            jti: String;
            subject: Uuid;
            purpose: String;
            expires_at: i64;
            revoked: bool = false;
            extra: Option<String>;
        }
        actions {
            create store { primary; accept [jti, subject, purpose, expires_at]; }
            read read { primary; }
        }
    }
}

fn migrations() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}

async fn seed_legacy<D: DataLayer>(data: D) {
    let ctx = Runtime::new(data).bootstrap();
    LegacyToken::store(&ctx)
        .jti("old-session")
        .subject(Uuid::new_v4())
        .purpose("refresh")
        .expires_at(4102444800_i64)
        .await
        .unwrap();
    assert_eq!(LegacyToken::query(&ctx).all().await.unwrap().len(), 1);
}

async fn write_new_session<D: TransactionSupport>(data: D) {
    let runtime = Runtime::new(data);
    let ctx = runtime.bootstrap();
    assert!(AshToken::query(&ctx).all().await.unwrap().is_empty());
    let store = DatabaseTokenStore::new(
        ctx,
        "User",
        runtime.credential_writes::<AshToken>(["revoked", "consumed"]),
    );
    let claims = Claims {
        sub: Uuid::new_v4(),
        resource: "User".into(),
        credential_version: 0,
        jti: "new-session".into(),
        purpose: "refresh".into(),
        exp: 4102444800,
        iat: 0,
        role: None,
        tenant: None,
    };
    store.store_token(&claims).await.unwrap();
    assert!(store.get_valid_token(&claims).await.unwrap().is_some());
    store.consume(&claims).await.unwrap();
    assert!(store.consume(&claims).await.is_err());
}

#[tokio::test]
async fn sqlite_token_upgrade_and_downgrade_invalidate_previous_sessions() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install(&[&LegacyToken::DEF]).await.unwrap();
    seed_legacy(data.clone()).await;
    assert_eq!(data.migrate(migrations()).await.unwrap().len(), 1);
    write_new_session(data.clone()).await;
    assert!(data.rollback(migrations()).await.unwrap().is_some());
    assert!(
        LegacyToken::query(&Runtime::new(data).bootstrap())
            .all()
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn postgres_token_upgrade_and_downgrade_invalidate_previous_sessions() {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL is required");
    let admin = ash_postgres::Postgres::connect(&url).await.unwrap();
    let schema = format!("token_upgrade_{}", Uuid::new_v4().simple());
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
    data.install(&[&LegacyToken::DEF]).await.unwrap();
    seed_legacy(data.clone()).await;
    assert_eq!(data.migrate(migrations()).await.unwrap().len(), 1);
    write_new_session(data.clone()).await;
    assert!(data.rollback(migrations()).await.unwrap().is_some());
    assert!(
        LegacyToken::query(&Runtime::new(data).bootstrap())
            .all()
            .await
            .unwrap()
            .is_empty()
    );
    admin
        .execute_sql(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
}
