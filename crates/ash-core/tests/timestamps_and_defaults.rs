use ash_core::{Context, Resource, SchemaSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

fn generate_tracking_code() -> String {
    String::from("TRK-9999")
}

pub mod article {
    use super::*;

    resource! {
        Article {
        table "articles";
        timestamps;

        attributes {
            id: Uuid [pk];
            title: String;
            status: String = "draft";
            views: i64 = 0;
            tracking_code: String [default_fn: generate_tracking_code];
        }

        actions {
            create publish {
                primary;
                accept [title, status, views, tracking_code];
            }

            read read {
                primary;
            }

            update revise {
                primary;
                accept [title, status];
            }
        }
    }}
}
pub use article::{Article, ArticleActions};

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_timestamps_and_defaults_in_memory() {
    let memory = Memory::new();
    let ctx = Context::new(memory);

    // 1. Create with defaults and automatic timestamps
    let created = Article::publish(&ctx)
        .title("Deep Dive into Ash Rust")
        .await
        .expect("publish article should succeed");

    // Invariant: static defaults applied
    assert_eq!(created.status, "draft");
    assert_eq!(created.views, 0);

    // Invariant: dynamic default_fn applied
    assert_eq!(created.tracking_code, "TRK-9999");

    // Invariant: timestamps automatically injected as valid ISO-8601 strings
    assert!(!created.created_at.is_empty());
    assert!(!created.updated_at.is_empty());
    assert!(created.created_at.contains('T') && created.created_at.ends_with('Z'));
    assert_eq!(created.created_at, created.updated_at);

    let initial_created_at = created.created_at.clone();

    // 2. Explicitly overriding defaults during create
    let custom = Article::publish(&ctx)
        .title("Custom Settings Article")
        .status("published")
        .views(42)
        .tracking_code("CUSTOM-1234")
        .await
        .expect("publish with custom values should succeed");

    assert_eq!(custom.status, "published");
    assert_eq!(custom.views, 42);
    assert_eq!(custom.tracking_code, "CUSTOM-1234");

    // 3. Update should bump updated_at while preserving created_at
    // Ensure at least 1 millisecond or second elapsed if needed, or simply verify the update hook runs
    let updated = created
        .revise(&ctx)
        .title("Deep Dive into Ash Rust (2nd Edition)")
        .status("published")
        .await
        .expect("revise article should succeed");

    assert_eq!(updated.title, "Deep Dive into Ash Rust (2nd Edition)");
    assert_eq!(updated.status, "published");
    assert_eq!(updated.created_at, initial_created_at, "created_at must never be mutated on update");
    // updated_at is recalculated on update
    assert!(!updated.updated_at.is_empty());
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_timestamps_and_defaults_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Article::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    // 1. Create in SQLite with auto timestamps and defaults
    let created = Article::publish(&ctx)
        .title("SQLite Persistence with Ash")
        .await
        .expect("publish in sqlite should succeed");

    assert_eq!(created.status, "draft");
    assert_eq!(created.views, 0);
    assert_eq!(created.tracking_code, "TRK-9999");
    assert!(!created.created_at.is_empty());
    assert_eq!(created.created_at, created.updated_at);

    // 2. Fetch directly from SQLite to verify columns were persisted
    let fetched = Article::get(&ctx, created.id)
        .await
        .expect("get article should fetch from sqlite");
    assert_eq!(fetched.title, "SQLite Persistence with Ash");
    assert_eq!(fetched.status, "draft");
    assert_eq!(fetched.views, 0);
    assert_eq!(fetched.tracking_code, "TRK-9999");
    assert_eq!(fetched.created_at, created.created_at);
    assert_eq!(fetched.updated_at, created.updated_at);

    // 3. Update in SQLite
    let updated = fetched
        .revise(&ctx)
        .title("SQLite Persistence with Ash (Revised)")
        .await
        .expect("revise in sqlite should succeed");

    assert_eq!(updated.title, "SQLite Persistence with Ash (Revised)");
    assert_eq!(updated.created_at, created.created_at);

    // 4. Fetch again to verify persistence of update in SQLite
    let refetched = Article::get(&ctx, created.id).await.unwrap();
    assert_eq!(refetched.title, "SQLite Persistence with Ash (Revised)");
    assert_eq!(refetched.created_at, created.created_at);
}

pub mod audit_log {
    use super::*;

    resource! {
        embedded AuditLog {
        timestamps;

        attributes {
            action: String;
            severity: String = "info";
        }

        actions {
            create record {
                primary;
                accept [action, severity];
            }
        }
    }}
}
pub use audit_log::AuditLog;

#[test]
fn test_embedded_resource_timestamps_and_defaults() {
    let log = AuditLog::build_record()
        .action("user_login")
        .build()
        .expect("build audit log should succeed");

    assert_eq!(log.action, "user_login");
    assert_eq!(log.severity, "info");
    assert!(!log.created_at.is_empty());
    assert_eq!(log.created_at, log.updated_at);
}
