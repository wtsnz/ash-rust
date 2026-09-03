use ash_core::{resource, Context, Resource, SchemaSupport};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod post {
    use super::*;

    resource! {
        resource Post;
        table "posts";

        attributes {
            id: Uuid [pk],
            title: String,
            status: String = "draft",
            views: i64 = 0,
            archived: bool = false,
        }

        actions {
            create create {
                primary;
                accept [title, status, views, archived];
            }

            // Primary read with default non-archived filter
            read read {
                primary;
                prepare filter(archived == false);
            }

            // Custom read action with default filter
            read published {
                prepare filter(status == "published");
                prepare filter(archived == false);
            }

            // Custom read action with default sort and limit
            read leaderboard {
                prepare filter(status == "published");
                prepare filter(archived == false);
                prepare sort(views, desc);
                prepare limit(2);
            }
        }
    }
}

pub use post::Post;

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_query_preparations_in_memory() {
    let data = Memory::new();
    let ctx = Context::new(data);

    // Create several posts
    Post::create(&ctx)
        .title("Draft Post")
        .status("draft")
        .views(10)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Published 1")
        .status("published")
        .views(150)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Published 2")
        .status("published")
        .views(500)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Published 3")
        .status("published")
        .views(300)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Archived Published")
        .status("published")
        .views(1000)
        .archived(true)
        .call()
        .await
        .unwrap();

    // Invariant 1: Primary read action automatically filters out archived posts
    let unarchived = Post::query(&ctx).all().await.unwrap();
    assert_eq!(unarchived.len(), 4);
    assert!(unarchived.iter().all(|p| !p.archived));
    assert_eq!(Post::query(&ctx).count().await.unwrap(), 4);

    // Invariant 2: Custom read action `published` only returns published & unarchived posts
    let published = Post::published(&ctx).all().await.unwrap();
    assert_eq!(published.len(), 3);
    assert!(published.iter().all(|p| p.status == "published" && !p.archived));
    assert_eq!(Post::published(&ctx).count().await.unwrap(), 3);

    // Invariant 3: Chaining user filter with prepared filters
    let popular_published = Post::published(&ctx)
        .filter(Post::views.gte(300))
        .all()
        .await
        .unwrap();
    assert_eq!(popular_published.len(), 2);
    assert!(popular_published.iter().all(|p| p.views >= 300));

    // Invariant 4: Prepared sort and limit on `leaderboard` action
    let leaders = Post::leaderboard(&ctx).all().await.unwrap();
    assert_eq!(leaders.len(), 2);
    assert_eq!(leaders[0].title, "Published 2");
    assert_eq!(leaders[0].views, 500);
    assert_eq!(leaders[1].title, "Published 3");
    assert_eq!(leaders[1].views, 300);

    // Invariant 5: User sort overrides prepared sort if specified
    let ascending_leaders = Post::leaderboard(&ctx)
        .sort_by(Post::views, false)
        .all()
        .await
        .unwrap();
    assert_eq!(ascending_leaders.len(), 2);
    assert_eq!(ascending_leaders[0].title, "Published 1"); // 150 views
    assert_eq!(ascending_leaders[1].title, "Published 3"); // 300 views

    // Invariant 6: Prepared first() helper
    let top_one = Post::leaderboard(&ctx).first().await.unwrap().unwrap();
    assert_eq!(top_one.title, "Published 2");

    // Invariant 7: Pagination with prepared filters
    let page = Post::published(&ctx).page_offset(2, 0, true).await.unwrap();
    assert_eq!(page.results.len(), 2);
    assert_eq!(page.total_count, Some(3));
    assert!(page.has_more);
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_query_preparations_in_sqlite() {
    let data = Sqlite::memory().await.unwrap();
    data.install_resources(&[&Post::DEF]).await.unwrap();
    let ctx = Context::new(data);

    // Create test posts
    Post::create(&ctx)
        .title("Draft Post")
        .status("draft")
        .views(10)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Published 1")
        .status("published")
        .views(150)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Published 2")
        .status("published")
        .views(500)
        .call()
        .await
        .unwrap();

    Post::create(&ctx)
        .title("Archived Published")
        .status("published")
        .views(1000)
        .archived(true)
        .call()
        .await
        .unwrap();

    // Invariant 1: Primary read action filters out archived posts
    let unarchived = Post::query(&ctx).all().await.unwrap();
    assert_eq!(unarchived.len(), 3);
    assert!(unarchived.iter().all(|p| !p.archived));
    assert_eq!(Post::query(&ctx).count().await.unwrap(), 3);

    // Invariant 2: Custom read action filters
    let published = Post::published(&ctx).all().await.unwrap();
    assert_eq!(published.len(), 2);
    assert!(published.iter().all(|p| p.status == "published" && !p.archived));

    // Invariant 3: Prepared sort & limit
    let leaders = Post::leaderboard(&ctx).all().await.unwrap();
    assert_eq!(leaders.len(), 2);
    assert_eq!(leaders[0].title, "Published 2");
    assert_eq!(leaders[0].views, 500);
    assert_eq!(leaders[1].title, "Published 1");
    assert_eq!(leaders[1].views, 150);
}
