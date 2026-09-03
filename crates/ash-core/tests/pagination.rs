use ash_core::{Context, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    resource Article;
    table "articles";

    attributes {
        id: Uuid [pk],
        title: String,
        views: i64,
    }

    actions {
        create create {
            primary;
            accept {
                title: String,
                views: i64,
            }
        }

        read read {
            primary;
        }
    }
}

#[tokio::test]
async fn test_offset_pagination_in_memory() {
    let ctx = Context::new(Memory::new());

    // Create 10 articles
    for i in 1..=10 {
        Article::create(&ctx)
            .title(format!("Article {i}"))
            .views(i * 10)
            .await
            .unwrap();
    }

    // Page 1: limit 3, offset 0, count_total true
    let page1 = Article::query(&ctx)
        .page_offset(3, 0, true)
        .await
        .expect("page 1 succeeds");

    assert_eq!(page1.results.len(), 3);
    assert!(page1.has_more);
    assert_eq!(page1.offset, Some(0));
    assert_eq!(page1.total_count, Some(10));
    assert_eq!(page1.limit, 3);

    // Page 2: limit 3, offset 3, count_total false
    let page2 = Article::query(&ctx)
        .page_offset(3, 3, false)
        .await
        .expect("page 2 succeeds");

    assert_eq!(page2.results.len(), 3);
    assert!(page2.has_more);
    assert_eq!(page2.offset, Some(3));
    assert_eq!(page2.total_count, None);

    // Page 4: limit 3, offset 9, count_total true (last item)
    let page4 = Article::query(&ctx)
        .page_offset(3, 9, true)
        .await
        .expect("page 4 succeeds");

    assert_eq!(page4.results.len(), 1);
    assert!(!page4.has_more);
    assert_eq!(page4.offset, Some(9));
    assert_eq!(page4.total_count, Some(10));
}

#[tokio::test]
async fn test_offset_pagination_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Article::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    for i in 1..=7 {
        Article::create(&ctx)
            .title(format!("SQLite Article {i}"))
            .views(i * 100)
            .await
            .unwrap();
    }

    let page1 = Article::query(&ctx)
        .page_offset(3, 0, true)
        .await
        .unwrap();

    assert_eq!(page1.results.len(), 3);
    assert!(page1.has_more);
    assert_eq!(page1.total_count, Some(7));

    let page2 = Article::query(&ctx)
        .page_offset(3, 3, true)
        .await
        .unwrap();

    assert_eq!(page2.results.len(), 3);
    assert!(page2.has_more);

    let page3 = Article::query(&ctx)
        .page_offset(3, 6, true)
        .await
        .unwrap();

    assert_eq!(page3.results.len(), 1);
    assert!(!page3.has_more);
}

#[tokio::test]
async fn test_keyset_pagination_in_memory() {
    let ctx = Context::new(Memory::new());

    for i in 1..=7 {
        Article::create(&ctx)
            .title(format!("Keyset Article {i}"))
            .views(i * 10)
            .await
            .unwrap();
    }

    // Page 1: first 3 records
    let page1 = Article::query(&ctx)
        .page_keyset(3, None, None)
        .await
        .expect("keyset page 1");

    assert_eq!(page1.results.len(), 3);
    assert!(page1.has_more);
    assert!(page1.after.is_some());
    let cursor1 = page1.after.as_deref().unwrap();

    // Page 2: next 3 records after cursor1
    let page2 = Article::query(&ctx)
        .page_keyset(3, Some(cursor1), None)
        .await
        .expect("keyset page 2");

    assert_eq!(page2.results.len(), 3);
    assert!(page2.has_more);
    assert!(page2.after.is_some());

    // Verify disjoint IDs
    let page1_ids: Vec<Uuid> = page1.results.iter().map(Resource::id).collect();
    for r in &page2.results {
        assert!(!page1_ids.contains(&r.id));
    }

    let cursor2 = page2.after.as_deref().unwrap();

    // Page 3: final record
    let page3 = Article::query(&ctx)
        .page_keyset(3, Some(cursor2), None)
        .await
        .expect("keyset page 3");

    assert_eq!(page3.results.len(), 1);
    assert!(!page3.has_more);
}

#[tokio::test]
async fn test_keyset_pagination_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Article::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    for i in 1..=5 {
        Article::create(&ctx)
            .title(format!("Sqlite Keyset {i}"))
            .views(i)
            .await
            .unwrap();
    }

    let page1 = Article::query(&ctx)
        .page_keyset(2, None, None)
        .await
        .unwrap();

    assert_eq!(page1.results.len(), 2);
    assert!(page1.has_more);
    let cursor1 = page1.after.as_deref().unwrap();

    let page2 = Article::query(&ctx)
        .page_keyset(2, Some(cursor1), None)
        .await
        .unwrap();

    assert_eq!(page2.results.len(), 2);
    assert!(page2.has_more);
    let cursor2 = page2.after.as_deref().unwrap();

    let page3 = Article::query(&ctx)
        .page_keyset(2, Some(cursor2), None)
        .await
        .unwrap();

    assert_eq!(page3.results.len(), 1);
    assert!(!page3.has_more);
}

#[tokio::test]
async fn test_keyset_pagination_with_custom_sort_and_before_in_memory() {
    let ctx = Context::new(Memory::new());

    // Create 6 articles with known views (including tie to test secondary PK tie-breaker)
    // 500, 400, 300 (A), 300 (B), 200, 100
    let a1 = Article::create(&ctx).title("Art 500").views(500).await.unwrap();
    let a2 = Article::create(&ctx).title("Art 400").views(400).await.unwrap();
    let a3 = Article::create(&ctx).title("Art 300-A").views(300).await.unwrap();
    let a4 = Article::create(&ctx).title("Art 300-B").views(300).await.unwrap();
    let a5 = Article::create(&ctx).title("Art 200").views(200).await.unwrap();
    let a6 = Article::create(&ctx).title("Art 100").views(100).await.unwrap();

    // Determine deterministic expected order: views DESC, then id DESC
    let mut all = [a1, a2, a3, a4, a5, a6];
    all.sort_by(|x, y| match y.views.cmp(&x.views) {
        std::cmp::Ordering::Equal => y.id.cmp(&x.id),
        other => other,
    });

    // Page 1 (limit 2, views DESC): should be all[0..2]
    let page1 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, None)
        .await
        .unwrap();

    assert_eq!(page1.results.len(), 2);
    assert_eq!(page1.results[0].id, all[0].id);
    assert_eq!(page1.results[1].id, all[1].id);
    assert!(page1.has_more);

    // Page 2 (after page1.after, limit 2): should be all[2..4]
    let cursor_p1_after = page1.after.as_deref().unwrap();
    let page2 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, Some(cursor_p1_after), None)
        .await
        .unwrap();

    assert_eq!(page2.results.len(), 2);
    assert_eq!(page2.results[0].id, all[2].id);
    assert_eq!(page2.results[1].id, all[3].id);
    assert!(page2.has_more);

    // Page 3 (after page2.after, limit 2): should be all[4..6]
    let cursor_p2_after = page2.after.as_deref().unwrap();
    let page3 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, Some(cursor_p2_after), None)
        .await
        .unwrap();

    assert_eq!(page3.results.len(), 2);
    assert_eq!(page3.results[0].id, all[4].id);
    assert_eq!(page3.results[1].id, all[5].id);
    assert!(!page3.has_more);

    // INVARIANT TEST FOR ISSUE 3:
    // Navigating backwards with `before` MUST return the immediately preceding page, NOT the earliest page!
    // From Page 3, requesting `before` page3.before (which is all[4]) with limit 2 MUST return Page 2 (all[2..4])!
    let cursor_p3_before = page3.before.as_deref().unwrap();
    let prev_page2 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, Some(cursor_p3_before))
        .await
        .unwrap();

    assert_eq!(prev_page2.results.len(), 2);
    assert_eq!(prev_page2.results[0].id, all[2].id, "must return immediately preceding page item 1");
    assert_eq!(prev_page2.results[1].id, all[3].id, "must return immediately preceding page item 2");
    assert!(prev_page2.has_more, "there are more preceding pages before page 2");

    // From Page 2, requesting `before` page2.before (all[2]) with limit 2 MUST return Page 1 (all[0..2])!
    let cursor_p2_before = prev_page2.before.as_deref().unwrap();
    let prev_page1 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, Some(cursor_p2_before))
        .await
        .unwrap();

    assert_eq!(prev_page1.results.len(), 2);
    assert_eq!(prev_page1.results[0].id, all[0].id);
    assert_eq!(prev_page1.results[1].id, all[1].id);
    assert!(!prev_page1.has_more, "no more pages before page 1");
}

#[tokio::test]
async fn test_keyset_pagination_with_custom_sort_and_before_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Article::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    let a1 = Article::create(&ctx).title("Sqlite 500").views(500).await.unwrap();
    let a2 = Article::create(&ctx).title("Sqlite 400").views(400).await.unwrap();
    let a3 = Article::create(&ctx).title("Sqlite 300-A").views(300).await.unwrap();
    let a4 = Article::create(&ctx).title("Sqlite 300-B").views(300).await.unwrap();
    let a5 = Article::create(&ctx).title("Sqlite 200").views(200).await.unwrap();
    let a6 = Article::create(&ctx).title("Sqlite 100").views(100).await.unwrap();

    let mut all = [a1, a2, a3, a4, a5, a6];
    all.sort_by(|x, y| match y.views.cmp(&x.views) {
        std::cmp::Ordering::Equal => y.id.cmp(&x.id),
        other => other,
    });

    let page1 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, None)
        .await
        .unwrap();

    assert_eq!(page1.results.len(), 2);
    assert_eq!(page1.results[0].id, all[0].id);
    assert_eq!(page1.results[1].id, all[1].id);
    assert!(page1.has_more);

    let cursor_p1_after = page1.after.as_deref().unwrap();
    let page2 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, Some(cursor_p1_after), None)
        .await
        .unwrap();

    assert_eq!(page2.results.len(), 2);
    assert_eq!(page2.results[0].id, all[2].id);
    assert_eq!(page2.results[1].id, all[3].id);
    assert!(page2.has_more);

    let cursor_p2_after = page2.after.as_deref().unwrap();
    let page3 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, Some(cursor_p2_after), None)
        .await
        .unwrap();

    assert_eq!(page3.results.len(), 2);
    assert_eq!(page3.results[0].id, all[4].id);
    assert_eq!(page3.results[1].id, all[5].id);
    assert!(!page3.has_more);

    // Backward navigation with before in SQLite
    let cursor_p3_before = page3.before.as_deref().unwrap();
    let prev_page2 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, Some(cursor_p3_before))
        .await
        .unwrap();

    assert_eq!(prev_page2.results.len(), 2);
    assert_eq!(prev_page2.results[0].id, all[2].id);
    assert_eq!(prev_page2.results[1].id, all[3].id);
    assert!(prev_page2.has_more);

    let cursor_p2_before = prev_page2.before.as_deref().unwrap();
    let prev_page1 = Article::query(&ctx)
        .sort_by(Article::views, true)
        .page_keyset(2, None, Some(cursor_p2_before))
        .await
        .unwrap();

    assert_eq!(prev_page1.results.len(), 2);
    assert_eq!(prev_page1.results[0].id, all[0].id);
    assert_eq!(prev_page1.results[1].id, all[1].id);
    assert!(!prev_page1.has_more);
}
