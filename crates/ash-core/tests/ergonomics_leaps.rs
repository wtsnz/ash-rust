use std::sync::Arc;
use uuid::Uuid;

use ash_core::{Context, Resource, ResourceExt, resource};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub, PubSubResourceExt};
use ash_sqlite::Sqlite;

resource! {
    Article {
        table "articles";

    attributes {
        id: Uuid [pk];
        title: String;
        content: String;
        views: i64;
        archived: bool;
        tag: Option<String>;
    }

    actions {
        read read {
            primary;
        }

        create publish {
            primary;
            // Leap 1: DRY Action `accept [field1, field2]` without repeating types!
            accept [title, content, tag];
            change set(views = 0);
            change set(archived = false);
        }

        update archive {
            // Leap 1: DRY Action `accept [field]` on update!
            accept [archived];
        }

        destroy delete {
            primary;
        }
    }
    }}

#[tokio::test]
async fn test_leap1_and_leap2_dry_accept_and_zero_import_operators() {
    let ctx = Context::new(Memory::new());

    // Leap 1: Action builder accepts values with types inferred from attributes!
    // Optional field `tag` accepts both `&str`, `Some(...)`, and `None`
    let art1 = Article::publish(&ctx)
        .title("Modern Ash in Rust")
        .content("Declarative resource modeling")
        .tag("rust")
        .await
        .expect("publish article 1");

    assert_eq!(art1.title, "Modern Ash in Rust");
    assert_eq!(art1.views, 0);
    assert!(!art1.archived);
    assert_eq!(art1.tag.as_deref(), Some("rust"));

    let art2 = Article::publish(&ctx)
        .title("Zero-Import Operators")
        .content("Filter without boilerplate imports")
        .tag(None)
        .await
        .expect("publish article 2");

    assert_eq!(art2.tag, None);

    // Leap 2: Zero-Import Field Operators directly on Article struct!
    // Notice: Article::title and Article::views require zero module imports!
    let matching = Article::query(&ctx)
        .filter(Article::title.eq("Modern Ash in Rust") & Article::views.gte(0))
        .all()
        .await
        .expect("query articles");

    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].id, art1.id);
}

#[tokio::test]
async fn test_leap3_typed_pubsub_ergonomics() {
    let pubsub = Arc::new(PubSub::new());

    // Leap 3: Fluent Context attachment via with_pubsub
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::clone(&pubsub));

    // Leap 3: Typed subscription streams on Resource and Record
    let mut sub_all = Article::subscribe_all(&pubsub);
    let mut sub_publish = Article::subscribe_action(&pubsub, "publish");

    // Publish an article
    let article = Article::publish(&ctx)
        .title("Event Streaming")
        .content("Async notifications")
        .tag("pubsub")
        .await
        .expect("publish");

    // Receive typed events
    let notif_all = sub_all.recv().await.expect("receive on sub_all");
    assert_eq!(notif_all.resource, "Article");
    assert_eq!(notif_all.action, "publish");
    assert_eq!(notif_all.id, article.id);

    let notif_publish = sub_publish.recv().await.expect("receive on sub_publish");
    assert_eq!(notif_publish.id, article.id);

    // Subscribe to a specific record instance
    let mut sub_record = article.subscribe(&pubsub, Some("archive"));

    // Archive the article
    let _ = article
        .archive_on(&ctx)
        .archived(true)
        .await
        .expect("archive");

    let notif_record = sub_record.recv().await.expect("receive on sub_record");
    assert_eq!(notif_record.action, "archive");
    assert_eq!(notif_record.id, article.id);
}

#[tokio::test]
async fn test_leap4_record_lifecycle_helpers() {
    let ctx = Context::new(Memory::new());

    let article = Article::publish(&ctx)
        .title("Lifecycle Helpers")
        .content("Reload and destroy")
        .tag("helpers")
        .await
        .expect("publish");

    // 1. Mutate record in data layer
    let _ = article
        .archive_on(&ctx)
        .archived(true)
        .await
        .expect("archive");

    // 2. Leap 4: record.reload(&ctx)
    let reloaded = article.reload(&ctx).await.expect("reload article");
    assert!(reloaded.archived);

    // 3. Leap 4: record.destroy(&ctx) using primary destroy action
    reloaded.destroy(&ctx).await.expect("destroy article");

    // Verify record is gone
    let query_res = Article::query(&ctx)
        .filter(Article::id.eq(article.id))
        .all()
        .await
        .expect("query");
    assert!(query_res.is_empty());
}

#[tokio::test]
async fn test_leap5_fluid_multi_in_memory() {
    let ctx = Context::new(Memory::new());

    // Leap 5: ctx.multi() with direct action builders (no .changeset(), no .unwrap()!)
    let results = ctx
        .multi()
        .create(
            "article1",
            Article::publish(&ctx)
                .title("Batch Post 1")
                .content("First post")
                .tag("multi"),
        )
        .create(
            "article2",
            Article::publish(&ctx)
                .title("Batch Post 2")
                .content("Second post")
                .tag("multi"),
        )
        .create_from("article3", |ctx, results| {
            let art1: &Article = results.get("article1").expect("get article1");
            Article::publish(ctx)
                .title(format!("Follow-up to {}", art1.title))
                .content("Third post")
                .tag("multi")
                .changeset()
        })
        .insert("count", 3_i64)
        .commit_without_transaction()
        .await
        .expect("commit multi pipeline");

    let art1: &Article = results.get("article1").expect("art1");
    let art2: &Article = results.get("article2").expect("art2");
    let art3: &Article = results.get("article3").expect("art3");
    let count: &i64 = results.get("count").expect("count");

    assert_eq!(art1.title, "Batch Post 1");
    assert_eq!(art2.title, "Batch Post 2");
    assert_eq!(art3.title, "Follow-up to Batch Post 1");
    assert_eq!(*count, 3);
}

#[tokio::test]
async fn test_leap5_fluid_multi_in_sqlite_transaction() {
    let data = Sqlite::memory().await.expect("sqlite init");
    let ctx = Context::new(data);
    ctx.install(&[&Article::DEF])
        .await
        .expect("install schema");

    // Leap 5: ctx.multi().commit() executes inside an atomic SQLite transaction!
    let results = ctx
        .multi()
        .create(
            "article1",
            Article::publish(&ctx)
                .title("SQLite Post 1")
                .content("First SQL post")
                .tag("sqlite"),
        )
        .create(
            "article2",
            Article::publish(&ctx)
                .title("SQLite Post 2")
                .content("Second SQL post")
                .tag("sqlite"),
        )
        .commit()
        .await
        .expect("commit sqlite multi");

    let art1: &Article = results.get("article1").expect("art1");
    let art2: &Article = results.get("article2").expect("art2");
    assert_eq!(art1.title, "SQLite Post 1");
    assert_eq!(art2.title, "SQLite Post 2");

    // Verify both were persisted to SQLite
    let all = Article::query(&ctx).all().await.expect("query all");
    assert_eq!(all.len(), 2);
}
