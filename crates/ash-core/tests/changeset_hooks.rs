use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use ash_core::{Context, Error, Resource, Result, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

resource! {
    resource Article;
    table "articles";

    attributes {
        id: Uuid [pk],
        title: String,
        body: String,
        slug: Option<String>,
        view_count: i64 [default: 0],
    }

    actions {
        read read {
            primary;
        }

        create create {
            primary;
            accept [title, body, slug, view_count];
        }

        update update {
            primary;
            accept [title, body, slug, view_count];
        }
    }
}

#[tokio::test]
async fn test_before_action_mutates_attributes() -> Result<()> {
    let ctx = Context::new(Memory::new());

    // Hook intercepts and auto-computes slug from title
    let article = Article::create(&ctx)
        .title("Ash Framework in Rust")
        .body("Lifecycle hooks allow ad-hoc changeset transformations.")
        .before_action(|cs| {
            if let Some(ash_core::Value::String(title)) = cs.get_attribute("title") {
                let slug = title.to_lowercase().replace(' ', "-");
                cs.change_attribute("slug", slug);
            }
            Ok(())
        })
        .await?;

    assert_eq!(article.title, "Ash Framework in Rust");
    assert_eq!(article.slug, Some("ash-framework-in-rust".into()));

    // Verify it was actually persisted with the mutated slug
    let loaded = Article::get(&ctx, article.id).await?;
    assert_eq!(loaded.slug, Some("ash-framework-in-rust".into()));

    Ok(())
}

#[tokio::test]
async fn test_before_action_can_abort_action() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let res = Article::create(&ctx)
        .title("Forbidden Title")
        .body("Some content")
        .before_action(|cs| {
            if let Some(ash_core::Value::String(t)) = cs.get_attribute("title")
                && t.contains("Forbidden")
            {
                return Err(Error::Invalid("Forbidden title not allowed".into()));
            }
            Ok(())
        })
        .await;

    assert!(matches!(res, Err(Error::Invalid(msg)) if msg.contains("Forbidden title not allowed")));

    // Ensure nothing was saved in the data layer
    let all = ash_core::query::<Article, _>(&ctx).all().await?;
    assert!(all.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_after_action_receives_persisted_record() -> Result<()> {
    let ctx = Context::new(Memory::new());
    let captured_id = Arc::new(Mutex::new(None));
    let captured_id_clone = Arc::clone(&captured_id);

    let article = Article::create(&ctx)
        .title("After Action Hook Test")
        .body("Validating after action hook receives record.")
        .after_action(move |record| {
            *captured_id_clone.lock().unwrap() = Some(record.id);
            // We can also mutate in-memory instance
            record.view_count = 42;
            Ok(())
        })
        .await?;

    assert_eq!(*captured_id.lock().unwrap(), Some(article.id));
    assert_eq!(article.view_count, 42);

    Ok(())
}

#[tokio::test]
async fn test_after_transaction_on_success_and_failure() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let success_called = Arc::new(AtomicBool::new(false));
    let success_called_clone = Arc::clone(&success_called);

    // Successful commit triggers after_transaction with Ok(&record)
    let article = Article::create(&ctx)
        .title("Success Post")
        .body("Test after transaction success.")
        .after_transaction(move |res| {
            if let Ok(rec) = res {
                assert_eq!(rec.title, "Success Post");
                success_called_clone.store(true, Ordering::SeqCst);
            }
        })
        .await?;

    assert!(success_called.load(Ordering::SeqCst));
    assert_eq!(article.title, "Success Post");

    // Failing action triggers after_transaction with Err(&error)
    let failure_called = Arc::new(AtomicBool::new(false));
    let failure_called_clone = Arc::clone(&failure_called);

    let err_res = Article::create(&ctx)
        .title("Fail Post")
        .body("Trigger failure in before_action.")
        .before_action(|_| Err(Error::Invalid("Intentional failure".into())))
        .after_transaction(move |res| {
            if let Err(err) = res {
                assert!(matches!(err, Error::Invalid(msg) if msg.contains("Intentional failure")));
                failure_called_clone.store(true, Ordering::SeqCst);
            }
        })
        .await;

    assert!(err_res.is_err());
    assert!(failure_called.load(Ordering::SeqCst));

    Ok(())
}

#[tokio::test]
async fn test_update_lifecycle_hooks() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let article = Article::create(&ctx)
        .title("Draft")
        .body("Initial body")
        .await?;

    let hook_ran = Arc::new(AtomicBool::new(false));
    let hook_ran_clone = Arc::clone(&hook_ran);

    let updated = article
        .update(&ctx)
        .title("Published")
        .before_action(|cs| {
            cs.change_attribute("body", "Updated body in before_action");
            Ok(())
        })
        .after_action(move |_rec| {
            hook_ran_clone.store(true, Ordering::SeqCst);
            Ok(())
        })
        .await?;

    assert!(hook_ran.load(Ordering::SeqCst));
    assert_eq!(updated.title, "Published");
    assert_eq!(updated.body, "Updated body in before_action");

    Ok(())
}

#[tokio::test]
async fn test_multiple_hooks_execution_order() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let order = Arc::new(Mutex::new(Vec::new()));

    let o1 = Arc::clone(&order);
    let o2 = Arc::clone(&order);
    let o3 = Arc::clone(&order);
    let o4 = Arc::clone(&order);

    let _article = Article::create(&ctx)
        .title("Ordered Hooks")
        .body("Content")
        .before_action(move |_| {
            o1.lock().unwrap().push("before_1");
            Ok(())
        })
        .before_action(move |_| {
            o2.lock().unwrap().push("before_2");
            Ok(())
        })
        .after_action(move |_| {
            o3.lock().unwrap().push("after_1");
            Ok(())
        })
        .after_transaction(move |_| {
            o4.lock().unwrap().push("after_tx");
        })
        .await?;

    let executed = order.lock().unwrap().clone();
    assert_eq!(executed, vec!["before_1", "before_2", "after_1", "after_tx"]);

    Ok(())
}

#[tokio::test]
async fn test_hooks_with_sqlite_backend() -> Result<()> {
    let sqlite = Sqlite::memory().await?;
    let ctx = Context::new(sqlite);
    ctx.install(&[&Article::DEF]).await?;

    let tx_count = Arc::new(AtomicUsize::new(0));
    let tx_count_clone = Arc::clone(&tx_count);

    let article = Article::create(&ctx)
        .title("SQLite Hooks")
        .body("Testing SQLite compatibility with hooks")
        .before_action(|cs| {
            cs.change_attribute("slug", "sqlite-hooks");
            Ok(())
        })
        .after_transaction(move |res| {
            if res.is_ok() {
                tx_count_clone.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await?;

    assert_eq!(article.slug, Some("sqlite-hooks".into()));
    assert_eq!(tx_count.load(Ordering::SeqCst), 1);

    let fetched = Article::get(&ctx, article.id).await?;
    assert_eq!(fetched.slug, Some("sqlite-hooks".into()));

    Ok(())
}
