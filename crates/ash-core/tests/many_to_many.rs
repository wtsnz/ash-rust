use ash_core::{Context, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod tag {
    use super::*;

    resource! {
        resource Tag;
        table "tags";

        attributes {
            id: Uuid [pk],
            name: String,
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }
        }
    }
}

pub mod post_tag {
    use super::*;

    resource! {
        resource PostTag;
        table "post_tags";

        attributes {
            id: Uuid [pk],
            post_id: Uuid,
            tag_id: Uuid,
        }

        actions {
            create create {
                primary;
                accept [post_id, tag_id];
            }

            read read {
                primary;
            }
        }
    }
}

pub mod post {
    use super::*;
    use super::post_tag::PostTag;
    use super::tag::Tag;

    resource! {
        resource Post;
        table "posts";

        attributes {
            id: Uuid [pk],
            title: String,
        }

        relationships {
            many_to_many tags: Tag [through: PostTag, source_fk: post_id, dest_fk: tag_id];
        }

        aggregates {
            tag_count: Option<i64> = count(tags);
            has_tags: Option<bool> = exists(tags);
        }

        actions {
            create create {
                primary;
                accept [title];
            }

            read read {
                primary;
            }
        }
    }
}

use post::Post;
use post_tag::PostTag;
use tag::Tag;

#[tokio::test]
async fn test_many_to_many_loading_and_aggregates_in_memory() {
    let ctx = Context::new(Memory::new());

    // 1. Create Posts
    let post1 = Post::create(&ctx).title("First Post").await.unwrap();
    let post2 = Post::create(&ctx).title("Second Post").await.unwrap();

    // 2. Create Tags
    let tag_rust = Tag::create(&ctx).name("rust").await.unwrap();
    let tag_ash = Tag::create(&ctx).name("ash").await.unwrap();
    let tag_elixir = Tag::create(&ctx).name("elixir").await.unwrap();

    // 3. Associate post1 -> ["rust", "ash"]
    PostTag::create(&ctx)
        .post_id(post1.id)
        .tag_id(tag_rust.id)
        .await
        .unwrap();
    PostTag::create(&ctx)
        .post_id(post1.id)
        .tag_id(tag_ash.id)
        .await
        .unwrap();

    // Associate post2 -> ["ash", "elixir"]
    PostTag::create(&ctx)
        .post_id(post2.id)
        .tag_id(tag_ash.id)
        .await
        .unwrap();
    PostTag::create(&ctx)
        .post_id(post2.id)
        .tag_id(tag_elixir.id)
        .await
        .unwrap();

    // 4. Query with eager load of many_to_many relationship
    let posts = Post::query(&ctx)
        .load_rel(post::fields::tags)
        .load_aggregate(post::fields::tag_count)
        .load_aggregate(post::fields::has_tags)
        .load()
        .await
        .expect("query posts with tags");

    assert_eq!(posts.len(), 2);

    let p1 = posts.iter().find(|p| p.id == post1.id).unwrap();
    assert_eq!(p1.tag_count, Some(2));
    assert_eq!(p1.has_tags, Some(true));
    assert!(p1.tags.is_loaded());
    let p1_tag_names: Vec<&str> = p1
        .tags
        .as_slice()
        .unwrap()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(p1_tag_names.len(), 2);
    assert!(p1_tag_names.contains(&"rust"));
    assert!(p1_tag_names.contains(&"ash"));

    let p2 = posts.iter().find(|p| p.id == post2.id).unwrap();
    assert_eq!(p2.tag_count, Some(2));
    assert_eq!(p2.has_tags, Some(true));
    assert!(p2.tags.is_loaded());
    let p2_tag_names: Vec<&str> = p2
        .tags
        .as_slice()
        .unwrap()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(p2_tag_names.len(), 2);
    assert!(p2_tag_names.contains(&"ash"));
    assert!(p2_tag_names.contains(&"elixir"));
}

#[tokio::test]
async fn test_many_to_many_loading_and_aggregates_in_sqlite() {
    let sqlite = Sqlite::memory().await.expect("sqlite memory");
    sqlite
        .install(&[&Tag::DEF, &PostTag::DEF, &Post::DEF])
        .await
        .expect("install tables");

    let ctx = Context::new(sqlite);

    // 1. Create Posts
    let post1 = Post::create(&ctx).title("Rust Ash").await.unwrap();
    let post2 = Post::create(&ctx).title("Empty Post").await.unwrap();

    // 2. Create Tags
    let tag_systems = Tag::create(&ctx).name("systems").await.unwrap();
    let tag_declarative = Tag::create(&ctx).name("declarative").await.unwrap();

    // 3. Associate post1 -> ["systems", "declarative"]
    PostTag::create(&ctx)
        .post_id(post1.id)
        .tag_id(tag_systems.id)
        .await
        .unwrap();
    PostTag::create(&ctx)
        .post_id(post1.id)
        .tag_id(tag_declarative.id)
        .await
        .unwrap();

    // 4. Query with eager load of many_to_many relationship + aggregates in SQLite
    let posts = Post::query(&ctx)
        .load_rel(post::fields::tags)
        .load_aggregate(post::fields::tag_count)
        .load_aggregate(post::fields::has_tags)
        .load()
        .await
        .expect("query posts in sqlite");

    assert_eq!(posts.len(), 2);

    let p1 = posts.iter().find(|p| p.id == post1.id).unwrap();
    assert_eq!(p1.tag_count, Some(2));
    assert_eq!(p1.has_tags, Some(true));
    assert!(p1.tags.is_loaded());
    let p1_tag_names: Vec<&str> = p1
        .tags
        .as_slice()
        .unwrap()
        .iter()
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(p1_tag_names.len(), 2);
    assert!(p1_tag_names.contains(&"systems"));
    assert!(p1_tag_names.contains(&"declarative"));

    let p2 = posts.iter().find(|p| p.id == post2.id).unwrap();
    assert_eq!(p2.tag_count, Some(0));
    assert_eq!(p2.has_tags, Some(false));
    assert!(p2.tags.is_loaded());
    assert!(p2.tags.as_slice().unwrap().is_empty());
}
