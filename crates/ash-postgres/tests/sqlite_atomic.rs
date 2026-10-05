//! SQLite updates atomically as AshSqlite does: as one statement where nothing in the
//! update must raise an error from the stored record (it can't `expr_error`), the lock
//! version a filter, so concurrent updates lose nothing; otherwise not atomically, which
//! an action requiring atomic updates refuses.

use ash_core::{Context, Error, FieldMap, Filter, Resource, resource};
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Counter {
        table "atomic_counters";

        attributes {
            id: Uuid [pk];
            hits: i64 [default: 0];
            version: i64 [default: 1];
        }

        actions {
            create create { primary; accept [hits]; }
            read read { primary; }

            update hit {
                change optimistic_lock(version);
                change atomic_update(hits, hits + 1);
            }

            // Checks what it sets, from the stored record: SQLite can't raise that in
            // the statement.
            update capped_hit {
                change atomic_update(hits, hits + 1);
                validate numericality(hits, max: 3);
            }

            update capped_hit_reading_first {
                require_atomic false;
                change atomic_update(hits, hits + 1);
                validate numericality(hits, max: 3);
            }
        }
    }
}

async fn sqlite() -> Context<Sqlite> {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Counter::DEF]).await.unwrap();
    Context::new(sqlite)
}

#[tokio::test]
async fn concurrent_updates_lose_nothing() {
    let ctx = sqlite().await;
    let counter = Counter::create(&ctx).await.unwrap();
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let (ctx, counter) = (ctx.clone(), counter.clone());
            tokio::spawn(async move { counter.hit(&ctx).await })
        })
        .collect();
    let mut stale = 0;
    for handle in handles {
        // The lock version filters each update: one that lost the race is stale.
        match handle.await.unwrap() {
            Ok(_) => {}
            Err(Error::StaleRecord { .. }) => stale += 1,
            Err(other) => panic!("{other:?}"),
        }
    }
    let stored = Counter::query(&ctx).filter(Filter::eq("id", counter.id)).one().await.unwrap();
    assert_eq!(stored.hits, 10 - stale);
    assert_eq!(stored.version, 1 + 10 - stale);
}

#[tokio::test]
async fn a_stale_version_is_refused() {
    let ctx = sqlite().await;
    let counter = Counter::create(&ctx).await.unwrap();
    counter.hit(&ctx).await.unwrap();
    // The record in hand still has the old version.
    let stale = counter.hit(&ctx).await;
    assert!(matches!(stale, Err(Error::StaleRecord { .. })), "{stale:?}");
}

#[tokio::test]
async fn what_must_raise_from_the_record_isnt_atomic() {
    let ctx = sqlite().await;
    let counter = Counter::create(&ctx).hits(3).await.unwrap();
    let by_id = |action: &str| {
        let action = Counter::DEF.action(action).unwrap();
        ash_core::update_dynamic_via(&ctx, &Counter::DEF, None, action, counter.id, FieldMap::new())
    };
    // By id, what it sets is the stored record's, which only the statement could check.
    let refused = by_id("capped_hit").await;
    assert!(matches!(refused, Err(Error::MustBeAtomic { .. })), "{refused:?}");
    // Read first, it checks the record as read.
    let invalid = by_id("capped_hit_reading_first").await;
    assert!(matches!(invalid, Err(Error::Validation { .. })), "{invalid:?}");
    // With the record in hand, what it sets is known, and checked before the statement.
    let known = counter.capped_hit(&ctx).await;
    assert!(matches!(known, Err(Error::Validation { .. })), "{known:?}");
}

mod memo {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Memo {
            table "atomic_memos";
    
            attributes {
                id: Uuid [pk];
                owner_id: Uuid;
                text: String;
            }
    
            actions {
                create create { primary; accept [owner_id, text]; }
                read read { primary; }
                update edit { accept [text]; }
            }
    
            policies {
                policy action_type(create) | action_type(read) {
                    authorize_if always;
                }
                policy action(edit) {
                    authorize_if relates_to(owner_id);
                }
            }
        }
    }
}
use memo::Memo;

#[tokio::test]
async fn write_policies_filter_the_statement() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Memo::DEF]).await.unwrap();
    let owner = Uuid::new_v4();
    let ctx = Context::new(sqlite).with_actor(ash_core::Actor::new(owner));
    let memo = Memo::create(&ctx).owner_id(owner).text("a").await.unwrap();
    let edit = Memo::DEF.action("edit").unwrap();
    let input = |text: &str| FieldMap::from([("text".to_string(), ash_core::Value::from(text))]);

    let edited = ash_core::update_dynamic_via(&ctx, &Memo::DEF, None, edit, memo.id, input("b")).await.unwrap();
    assert_eq!(edited.get("text"), Some(&ash_core::Value::from("b")));
    // A stranger's update changes nothing, and is refused as the policies refuse it.
    let stranger = ctx.with_actor(ash_core::Actor::new(Uuid::new_v4()));
    let refused = ash_core::update_dynamic_via(&stranger, &Memo::DEF, None, edit, memo.id, input("c")).await;
    assert!(matches!(refused, Err(Error::Forbidden)), "{refused:?}");
}
