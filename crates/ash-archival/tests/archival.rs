use ash_archival::{ArchiveDef, archival, archive_def, unarchive};
use ash_core::{CompiledQuery, Context, DataLayer, Resource, ResourceExt, destroy_existing};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

pub mod post_mod {
    use super::*;
    use uuid::Uuid;

    #[archival]
    resource! {
        Post {
            table "posts";

            attributes {
                id: Uuid [pk];
                title: String;
            }

            relationships {
                has_many comments: super::comment_mod::Comment [fk: post_id];
            }

            aggregates {
                comment_count: Option<i64> = count(comments);
                has_comments: Option<bool> = exists(comments);
            }

            archive {
                exclude_read_actions [archived];
                exclude_destroy_actions [purge];
                archive_related [comments];
                unarchive_action unarchive;
            }

            actions {
                create create {
                    primary;
                    accept [title];
                }

                read read {
                    primary;
                }

                /// Archived posts only.
                read archived {
                    prepare filter(!archived_at.is_nil());
                }

                destroy destroy {
                    primary;
                }

                destroy purge;
            }
        }
    }
}

pub mod comment_mod {
    use super::*;
    use uuid::Uuid;

    #[archival]
    resource! {
        Comment {
            table "comments";

            attributes {
                id: Uuid [pk];
                post_id: Uuid;
                body: String;
            }

            relationships {
                belongs_to post: super::post_mod::Post [fk: post_id];
            }

            actions {
                create create {
                    primary;
                    accept [post_id, body];
                }

                read read {
                    primary;
                }

                destroy destroy {
                    primary;
                }
            }
        }
    }
}

use comment_mod::Comment;
use post_mod::Post;

async fn stored_rows<D: DataLayer>(ctx: &Context<D>, resource: &ash_core::ResourceDef) -> usize {
    ctx.data
        .run_query(resource, &CompiledQuery::default())
        .await
        .unwrap()
        .len()
}

async fn scenario<D: DataLayer>(ctx: Context<D>) {
    let kept = Post::create(&ctx).title("Kept").await.unwrap();
    let archived = Post::create(&ctx).title("Archived").await.unwrap();
    for (post, body) in [(&kept, "stays"), (&archived, "goes"), (&archived, "goes too")] {
        Comment::create(&ctx).post_id(post.id).body(body).await.unwrap();
    }

    // Destroy archives the post and, through archive_related, its comments.
    archived.destroy(&ctx).await.unwrap();
    assert_eq!(stored_rows(&ctx, &Post::DEF).await, 2, "the row must stay");
    let titles: Vec<String> = Post::query(&ctx)
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|post| post.title)
        .collect();
    assert_eq!(titles, ["Kept"]);
    assert_eq!(Comment::query(&ctx).count().await.unwrap(), 1);
    assert_eq!(stored_rows(&ctx, &Comment::DEF).await, 3);

    // The excluded read action still sees it.
    let found = Post::query(&ctx).action("archived").one().await.unwrap();
    assert_eq!(found.id, archived.id);
    assert!(found.is_archived());

    // Relationship loads use the primary read, so they skip archived comments.
    let loaded = Post::query(&ctx)
        .load_rel(Post::comments)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.comments.loaded().unwrap().len(), 1);

    // Aggregates and filters through a relationship use the primary read too, so
    // archived comments are neither counted nor matched.
    let mut counted: Vec<(String, Option<i64>, Option<bool>)> = Post::query(&ctx)
        .action("archived")
        .load_aggregate(Post::comment_count)
        .load_aggregate(Post::has_comments)
        .all()
        .await
        .unwrap()
        .into_iter()
        .chain(
            Post::query(&ctx)
                .load_aggregate(Post::comment_count)
                .load_aggregate(Post::has_comments)
                .all()
                .await
                .unwrap(),
        )
        .map(|post| (post.title, post.comment_count, post.has_comments))
        .collect();
    counted.sort();
    assert_eq!(
        counted,
        [
            ("Archived".to_string(), Some(0), Some(false)),
            ("Kept".to_string(), Some(1), Some(true)),
        ]
    );
    let matched = Post::query(&ctx)
        .action("archived")
        .filter(ash_core::Filter::related(
            "comments",
            ash_core::Filter::eq("body", "goes"),
        ))
        .count()
        .await
        .unwrap();
    assert_eq!(matched, 0, "archived comments must not match a related filter");
    let matched = Post::query(&ctx)
        .filter(ash_core::Filter::related(
            "comments",
            ash_core::Filter::eq("body", "stays"),
        ))
        .count()
        .await
        .unwrap();
    assert_eq!(matched, 1);

    // Unarchive restores the post. Its comments stay archived, as in AshArchival.
    let restored = unarchive::<Post, D>(&ctx, archived.id).await.unwrap();
    assert!(!restored.is_archived());
    assert_eq!(Post::query(&ctx).count().await.unwrap(), 2);
    assert_eq!(Comment::query(&ctx).count().await.unwrap(), 1);

    // An excluded destroy action really deletes.
    let scratch = Post::create(&ctx).title("Scratch").await.unwrap();
    destroy_existing::<Post, D>(&ctx, "purge", scratch).await.unwrap();
    assert_eq!(stored_rows(&ctx, &Post::DEF).await, 2);
}

#[test]
fn archive_settings_are_registered() {
    assert_eq!(
        archive_def::<Post>(),
        Some(&ArchiveDef::new(
            "archived_at",
            &["archived"],
            &["purge"],
            &["comments"],
            Some("unarchive"),
        ))
    );
    assert_eq!(archive_def::<Comment>().unwrap().attribute, "archived_at");
    let purge = Post::DEF.action("purge").unwrap();
    assert!(!purge.soft);
    let destroy = Post::DEF.action("destroy").unwrap();
    assert!(destroy.soft);
    assert_eq!(destroy.cascade_destroy, ["comments"]);
    assert!(Post::DEF.attribute("archived_at").unwrap().allow_nil);
}

#[tokio::test]
async fn archival_in_memory() {
    scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn archival_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Post::DEF, &Comment::DEF]).await.unwrap();
    scenario(Context::new(sqlite)).await;
}

#[tokio::test]
async fn graphql_destroy_archives_and_lists_hide_archived() {
    use async_graphql::Request;

    let ctx = Context::new(Memory::new());
    let post = Post::create(&ctx).title("Via GraphQL").await.unwrap();
    let schema = ash_graphql::AshGraphQL::from_resources(&[&Post::DEF, &Comment::DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    let destroy = format!(
        r#"mutation {{ destroyPost(input: {{ id: "{}" }}) {{ success errors {{ message }} }} }}"#,
        post.id
    );
    let res = schema.execute(Request::new(destroy).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let data = res.data.into_json().unwrap();
    assert_eq!(data["destroyPost"]["success"], true, "{data}");

    let res = schema
        .execute(Request::new("query { listPosts { id archived_at } }").data(ctx.clone()))
        .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(res.data.into_json().unwrap()["listPosts"], serde_json::json!([]));
    assert_eq!(stored_rows(&ctx, &Post::DEF).await, 1);
    assert!(Post::query(&ctx).action("archived").one().await.unwrap().is_archived());
}
