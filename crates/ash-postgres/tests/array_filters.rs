//! A list attribute holds its items as their own type, and `has` finds the records whose
//! list holds a value, as Ash's `has(tags, "urgent")`, in every data layer.

use ash_core::{Context, DataLayer, Filter, Resource, input, resource};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use serde_json::json;
use uuid::Uuid;

resource! {
    Post {
        table "list_filter_posts";

        attributes {
            id: Uuid [pk];
            title: String;
            run: Uuid;
            tags: Vec<String> [default: Vec::new()];
            scores: Vec<i64> [default: Vec::new()];
        }

        actions {
            create create { primary; accept [title, run, tags, scores]; }
            read read { primary; }
        }
    }
}

/// The titles of this run's posts (Postgres keeps earlier runs') matching `filter`.
async fn titles<D: DataLayer>(ctx: &Context<D>, run: Uuid, filter: Filter) -> Vec<String> {
    let mut titles: Vec<String> =
        Post::query(ctx).filter(Filter::eq("run", run) & filter).all().await.unwrap().into_iter().map(|post| post.title).collect();
    titles.sort();
    titles
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    let ctx = Context::new(data);
    let run = Uuid::new_v4();
    let tags = |tags: &[&str]| tags.iter().map(|tag| tag.to_string()).collect::<Vec<_>>();
    Post::create(&ctx).title("a").run(run).tags(tags(&["urgent", "billing"])).scores(vec![1, 5]).await.unwrap();
    Post::create(&ctx).title("b").run(run).tags(tags(&["billing"])).scores(vec![5]).await.unwrap();
    Post::create(&ctx).title("c").run(run).await.unwrap();

    let read = Post::query(&ctx).filter(Filter::eq("run", run) & Filter::eq("title", "a")).one().await.unwrap();
    assert_eq!(read.tags, tags(&["urgent", "billing"]));
    assert_eq!(read.scores, vec![1, 5]);

    assert_eq!(titles(&ctx, run, Filter::has("tags", "urgent")).await, ["a"]);
    assert_eq!(titles(&ctx, run, Filter::has("tags", "billing")).await, ["a", "b"]);
    assert_eq!(titles(&ctx, run, Filter::has("scores", 5i64)).await, ["a", "b"]);
    assert!(titles(&ctx, run, Filter::has("scores", 2i64)).await.is_empty());
    assert_eq!(titles(&ctx, run, !Filter::has("tags", "billing")).await, ["c"]);

    // A client's filter, with the value cast to the list's item type.
    let filter = input::filter_input(&Post::DEF, &json!({ "scores": { "has": 1 } })).unwrap();
    assert_eq!(titles(&ctx, run, filter).await, ["a"]);
    let wrong = input::filter_input(&Post::DEF, &json!({ "title": { "has": "a" } }));
    assert!(wrong.is_err(), "{wrong:?}");
}

#[tokio::test]
async fn list_filters_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn list_filters_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Post::DEF]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn list_filters_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&Post::DEF]).await.unwrap();
    scenario(pg).await;
}
