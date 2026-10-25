#[path = "../../ash-core/tests/support/trusted_nodes.rs"]
mod support;
use ash_core::{Error, FieldMap, ReadLock, ReadRequest, Resource, Runtime, Value};
use ash_postgres::Postgres;
use support::*;
use uuid::Uuid;

async fn database() -> Postgres {
    let url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL is required for trusted plan node tests");
    let data = Postgres::connect(&url).await.unwrap();
    data.install(RESOURCES).await.unwrap();
    data
}

#[tokio::test]
async fn postgres_fragments_filter_sort_and_calculate_with_tenancy_and_policies() {
    let data = database().await;
    let result: ash_core::Result<()> = ash_core::TransactionSupport::transaction(&data, |data| {
        let runtime = Runtime::new(data.clone());
        async move {
            fragment_results(runtime).await;
            Err(Error::Invalid("rollback fixture".into()))
        }
    })
    .await;
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn postgres_distinct_winners_stay_stable_across_forward_and_backward_sealed_pages() {
    let data = database().await;
    let runtime = Runtime::new(data);
    let result: ash_core::Result<()> = runtime
        .transaction(|runtime| async move {
            seed(&runtime).await;
            let ctx = reader(&runtime);
            let query = Document::query(&ctx)
                .sort_desc(Document::id)
                .distinct_on(["category"]);
            let first = query.clone().page_keyset(1, None, None).await.unwrap();
            assert_eq!(first.results[0].id, Uuid::from_u128(6));
            let second = query
                .clone()
                .page_keyset(1, first.after.as_deref(), None)
                .await
                .unwrap();
            assert_eq!(second.results[0].id, Uuid::from_u128(3));
            let third = query
                .clone()
                .page_keyset(1, second.after.as_deref(), None)
                .await
                .unwrap();
            assert_eq!(third.results[0].id, Uuid::from_u128(2));
            assert!(!third.has_more);
            let end = query
                .clone()
                .page_keyset(1, third.after.as_deref(), None)
                .await
                .unwrap();
            assert!(end.results.is_empty(), "losing row 1 must not reappear");
            let previous = query
                .clone()
                .page_keyset(1, None, third.before.as_deref())
                .await
                .unwrap();
            assert_eq!(previous.results[0].id, Uuid::from_u128(3));
            assert_eq!(query.clone().count().await.unwrap(), 3);
            assert!(
                Document::query(&ctx)
                    .sort_desc(Document::id)
                    .distinct_on(["title"])
                    .page_keyset(1, first.after.as_deref(), None)
                    .await
                    .is_err(),
                "a cursor cannot change its distinct groups"
            );
            let views = ctx
                .access()
                .read(
                    &Document::DEF,
                    "read",
                    &FieldMap::new(),
                    ReadRequest {
                        distinct_on: Some(ash_core::DistinctOn {
                            fields: vec!["category".into()],
                            sort: vec![ash_core::Sort {
                                field: "category".into(),
                                guard: Some(ash_core::Filter::eq("id", Uuid::from_u128(2))),
                                ..Default::default()
                            }],
                        }),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(views.len(), 3);
            let chosen = views
                .iter()
                .find(|row| row.get("category") == Some(&Value::String("a".into())))
                .unwrap();
            assert_eq!(
                chosen.get("id"),
                Some(&Value::Uuid(Uuid::from_u128(2))),
                "guarded group-field sorts still select the winner"
            );
            assert!(
                views
                    .iter()
                    .all(|row| row.get("secret") == Some(&Value::Null))
            );
            Err(Error::Invalid("rollback fixture".into()))
        })
        .await;
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn postgres_full_text_preparations_page_by_hidden_relevance_with_sealed_cursors() {
    let data = database().await;
    let runtime = Runtime::new(data).with_cursor_key([73; 32]);
    let result: ash_core::Result<()> = runtime
        .transaction(|runtime| async move {
            seed(&runtime).await;
            let ctx = reader(&runtime);
            let query = Document::query(&ctx)
                .action("search")
                .argument("term", "rust")
                .calc_with_args(Document::relevance, ash_core::fields! {"term" => "rust"});
            let distinct = query.clone().distinct_on(["category"]);
            assert_eq!(distinct.clone().count().await.unwrap(), 2);
            assert_eq!(
                distinct.page_offset(1, 0, true).await.unwrap().total_count,
                Some(2)
            );
            let first = query.clone().page_keyset(1, None, None).await.unwrap();
            assert_eq!(first.results[0].title, "Rust rust rust");
            assert_eq!(first.results[0].relevance, None);
            let second = query
                .clone()
                .page_keyset(1, first.after.as_deref(), None)
                .await
                .unwrap();
            let third = query
                .clone()
                .page_keyset(1, second.after.as_deref(), None)
                .await
                .unwrap();
            assert_ne!(second.results[0].id, third.results[0].id);
            assert!(!third.has_more);
            assert!(first.after.as_ref().unwrap().starts_with("v1."));
            let mut tampered = first.after.unwrap().into_bytes();
            tampered[8] = if tampered[8] == b'A' { b'B' } else { b'A' };
            assert!(
                query
                    .page_keyset(1, Some(std::str::from_utf8(&tampered).unwrap()), None)
                    .await
                    .is_err()
            );
            Err(Error::Invalid("rollback fixture".into()))
        })
        .await;
    assert!(matches!(result, Err(Error::Invalid(_))));
}

#[tokio::test]
async fn postgres_locks_require_transactions_and_skip_rows_locked_by_another_transaction() {
    let data = database().await;
    let runtime = Runtime::new(data.clone());
    let ctx = runtime.bind(None, Some("locking".into()));
    assert!(matches!(
        Document::query(&ctx).for_update().all().await,
        Err(Error::LockRequiresTransaction)
    ));
    let completed = ctx.transaction(|tx| async move { Ok(tx) }).await.unwrap();
    assert!(matches!(
        Document::query(&completed).for_update().all().await,
        Err(Error::LockRequiresTransaction)
    ));
    // This row is committed so two separate transactions can see it.
    let id = Uuid::new_v4();
    Document::create(&ctx)
        .id(id)
        .title("locked")
        .category("lock")
        .secret("s")
        .enabled(true)
        .await
        .unwrap();
    let cleanup_pool = data.pool().unwrap().clone();
    let result = ctx
        .transaction(|locked| async move {
            let rows = Document::query(&locked)
                .filter(Document::id.eq(id))
                .for_update()
                .all()
                .await?;
            assert_eq!(rows.len(), 1);
            let page = Document::query(&locked)
                .filter(Document::id.eq(id))
                .for_update()
                .page_offset(1, 0, true)
                .await?;
            assert_eq!(page.total_count, Some(1));
            assert_eq!(page.results.len(), 1);
            let other = Runtime::new(data.clone());
            let other_ctx = other.bind(None, Some("locking".into()));
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                other_ctx.transaction(|other| async move {
                    let rows = Document::query(&other)
                        .filter(Document::id.eq(id))
                        .lock(ReadLock::ForUpdateSkipLocked)
                        .all()
                        .await?;
                    assert!(rows.is_empty());
                    Ok(())
                }),
            )
            .await
            .map_err(|_| Error::Invalid("skip_locked waited for a locked row".into()))??;
            Ok(())
        })
        .await;
    // Clean up only the committed concurrency fixture.
    cleanup_pool
        .get()
        .await
        .unwrap()
        .execute(
            "DELETE FROM trusted_node_documents WHERE id = $1 AND tenant = $2",
            &[&id, &"locking"],
        )
        .await
        .unwrap();
    result.unwrap();
}
