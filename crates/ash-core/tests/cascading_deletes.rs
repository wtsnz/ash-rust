use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;

use ash_core::{ActionKind, Context, Error, Resource, ResourceExt, Result, SyncFnNotifier, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

pub mod author_mod {
    use super::*;

    resource! {
        Author {
        table "authors";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        relationships {
            has_many posts: post_mod::Post [fk: author_id, on_delete: cascade];
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod post_mod {
    use super::*;

    resource! {
        Post {
        table "posts";

        attributes {
            id: Uuid [pk];
            author_id: Uuid;
            title: String;
        }

        relationships {
            belongs_to author: author_mod::Author [fk: author_id];
            has_many comments: comment_mod::Comment [fk: post_id, on_delete: cascade];
        }

        actions {
            create create {
                primary;
                accept [author_id, title];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod comment_mod {
    use super::*;

    resource! {
        Comment {
        table "comments";

        attributes {
            id: Uuid [pk];
            post_id: Uuid;
            content: String;
        }

        relationships {
            belongs_to post: post_mod::Post [fk: post_id];
        }

        actions {
            create create {
                primary;
                accept [post_id, content];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod dept_mod {
    use super::*;

    resource! {
        Department {
        table "departments";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        relationships {
            has_many employees: emp_mod::Employee [fk: dept_id, on_delete: restrict];
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod emp_mod {
    use super::*;

    resource! {
        Employee {
        table "employees";

        attributes {
            id: Uuid [pk];
            dept_id: Uuid;
            name: String;
        }

        relationships {
            belongs_to department: dept_mod::Department [fk: dept_id];
        }

        actions {
            create create {
                primary;
                accept [dept_id, name];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod team_mod {
    use super::*;

    resource! {
        Team {
        table "teams";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        relationships {
            has_many players: player_mod::Player [fk: team_id, on_delete: nilify];
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod player_mod {
    use super::*;

    resource! {
        Player {
        table "players";

        attributes {
            id: Uuid [pk];
            team_id: Option<Uuid>;
            name: String;
        }

        relationships {
            belongs_to team: team_mod::Team [fk: team_id];
        }

        actions {
            create create {
                primary;
                accept [team_id, name];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod tag_mod {
    use super::*;

    resource! {
        Tag {
        table "tags";

        attributes {
            id: Uuid [pk];
            label: String;
        }

        actions {
            create create {
                primary;
                accept [label];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod join_mod {
    use super::*;

    resource! {
        ArticleTag {
        table "article_tags";

        attributes {
            id: Uuid [pk];
            article_id: Uuid;
            tag_id: Uuid;
        }

        actions {
            create create {
                primary;
                accept [article_id, tag_id];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod article_mod {
    use super::*;
    use super::join_mod::ArticleTag;
    use super::tag_mod::Tag;

    resource! {
        Article {
        table "articles";

        attributes {
            id: Uuid [pk];
            title: String;
        }

        relationships {
            many_to_many tags: Tag [
                through: ArticleTag,
                source_fk: article_id,
                dest_fk: tag_id,
                on_delete: cascade,
            ];
        }

        actions {
            create create {
                primary;
                accept [title];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
            }
        }
    }}
}

use article_mod::Article;
use author_mod::Author;
use comment_mod::Comment;
use dept_mod::Department;
use emp_mod::Employee;
use join_mod::ArticleTag;
use player_mod::Player;
use post_mod::Post;
use tag_mod::Tag;
use team_mod::Team;

#[tokio::test]
async fn test_cascading_delete_multi_level_in_memory() -> Result<()> {
    let destroy_count = Arc::new(AtomicUsize::new(0));
    let count_clone = Arc::clone(&destroy_count);
    let notifier = Arc::new(SyncFnNotifier::new("test_counter", move |notif| {
        if notif.action_kind == ActionKind::Destroy {
            count_clone.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }));

    let ctx = Context::new(Memory::new()).with_notifier(notifier);

    let author = Author::create(&ctx).name("J.K. Rowling").await?;

    let post1 = Post::create(&ctx).author_id(author.id).title("Book 1").await?;
    let post2 = Post::create(&ctx).author_id(author.id).title("Book 2").await?;

    let _c1 = Comment::create(&ctx).post_id(post1.id).content("Great!").await?;
    let _c2 = Comment::create(&ctx).post_id(post1.id).content("Loved it!").await?;
    let _c3 = Comment::create(&ctx).post_id(post2.id).content("Awesome!").await?;

    // Verify setup
    assert_eq!(Post::query(&ctx).all().await?.len(), 2);
    assert_eq!(Comment::query(&ctx).all().await?.len(), 3);

    // Destroy author -> should cascade to 2 posts and 3 comments
    author.destroy(&ctx).await?;

    // Check all child and grandchild records are deleted
    assert!(Author::query(&ctx).all().await?.is_empty());
    assert!(Post::query(&ctx).all().await?.is_empty());
    assert!(Comment::query(&ctx).all().await?.is_empty());

    // Total destroy actions executed: 1 author + 2 posts + 3 comments = 6
    assert_eq!(destroy_count.load(Ordering::SeqCst), 6);

    Ok(())
}

#[tokio::test]
async fn test_restrict_delete_prevents_parent_deletion() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let engineering = Department::create(&ctx).name("Engineering").await?;
    let emp = Employee::create(&ctx).dept_id(engineering.id).name("Alice").await?;

    // Attempting to delete engineering must fail because it has 1 dependent employee
    let res = engineering.destroy(&ctx).await;
    match res {
        Err(Error::DeleteRestricted { resource, relationship, count }) => {
            assert_eq!(resource, "Department");
            assert_eq!(relationship, "employees");
            assert_eq!(count, 1);
        }
        other => panic!("expected DeleteRestricted, got {other:?}"),
    }

    // Engineering still exists
    assert!(Department::get(&ctx, engineering.id).await.is_ok());

    // Once the employee is removed, deletion succeeds
    emp.destroy(&ctx).await?;
    engineering.destroy(&ctx).await?;

    assert!(Department::query(&ctx).all().await?.is_empty());

    Ok(())
}

#[tokio::test]
async fn test_nilify_delete_clears_foreign_key() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let team = Team::create(&ctx).name("Warriors").await?;
    let player1 = Player::create(&ctx).team_id(Some(team.id)).name("Curry").await?;
    let player2 = Player::create(&ctx).team_id(Some(team.id)).name("Thompson").await?;

    // Destroy team -> players should have team_id set to None
    team.destroy(&ctx).await?;

    assert!(Team::query(&ctx).all().await?.is_empty());

    let p1_after = Player::get(&ctx, player1.id).await?;
    let p2_after = Player::get(&ctx, player2.id).await?;

    assert_eq!(p1_after.team_id, None);
    assert_eq!(p2_after.team_id, None);
    assert_eq!(p1_after.name, "Curry");

    Ok(())
}

#[tokio::test]
async fn test_many_to_many_cascades_join_rows() -> Result<()> {
    let ctx = Context::new(Memory::new());

    let article = Article::create(&ctx).title("Rust Async").await?;
    let tag_rust = Tag::create(&ctx).label("rust").await?;
    let tag_async = Tag::create(&ctx).label("async").await?;

    ArticleTag::create(&ctx).article_id(article.id).tag_id(tag_rust.id).await?;
    ArticleTag::create(&ctx).article_id(article.id).tag_id(tag_async.id).await?;

    assert_eq!(ArticleTag::query(&ctx).all().await?.len(), 2);

    // Destroy article -> cascades to ArticleTag join rows, but tags remain!
    article.destroy(&ctx).await?;

    assert!(Article::query(&ctx).all().await?.is_empty());
    assert!(ArticleTag::query(&ctx).all().await?.is_empty());

    // Tags themselves still exist
    assert_eq!(Tag::query(&ctx).all().await?.len(), 2);

    Ok(())
}

#[tokio::test]
async fn test_cascading_delete_in_sqlite() -> Result<()> {
    let sqlite = Sqlite::memory().await?;
    let ctx = Context::new(sqlite);
    ctx.install(&[&Author::DEF, &Post::DEF, &Comment::DEF]).await?;

    let author = Author::create(&ctx).name("George R.R. Martin").await?;
    let post = Post::create(&ctx).author_id(author.id).title("A Game of Thrones").await?;
    let _comm = Comment::create(&ctx).post_id(post.id).content("Winter is coming").await?;

    assert_eq!(Comment::query(&ctx).all().await?.len(), 1);

    author.destroy(&ctx).await?;

    assert!(Author::query(&ctx).all().await?.is_empty());
    assert!(Post::query(&ctx).all().await?.is_empty());
    assert!(Comment::query(&ctx).all().await?.is_empty());

    Ok(())
}
