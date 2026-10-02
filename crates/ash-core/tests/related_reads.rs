//! A relationship load reads the destination through its default read: the read's
//! filters hide rows, its sort orders each parent's rows, and its limit pages each
//! parent's rows rather than the whole batch.

use ash_core::{Context, DataLayer, Resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

use author::Author;
use post::Post;

pub mod author {
    use super::post::Post;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Author {
            table "authors";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many posts: Post [fk: author_id];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
            }
        }
    }
}

pub mod post {
    use super::author::Author;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        /// Each author's two latest live posts.
        Post {
            table "posts";

            attributes {
                id: Uuid [pk];
                author_id: Uuid;
                title: String;
                position: i64;
                archived: bool = false;
            }

            relationships {
                belongs_to author: Author [fk: author_id];
            }

            actions {
                create create { primary; accept [author_id, title, position, archived]; }
                read read {
                    primary;
                    prepare filter(archived == false);
                    prepare sort(position, desc);
                    prepare limit(2);
                }
            }
        }
    }
}

async fn loads_page_each_parent<D: DataLayer>(data: D) {
    let ctx = Context::new(data);
    for name in ["Ada", "Grace"] {
        let author = Author::create(&ctx).name(name).await.unwrap();
        for position in 1..=4 {
            Post::create(&ctx)
                .author_id(author.id)
                .title(format!("{name} {position}"))
                .position(position)
                .archived(position == 4)
                .await
                .unwrap();
        }
    }

    let authors = Author::query(&ctx)
        .load_rel(Author::posts)
        .sort(Author::name)
        .all()
        .await
        .unwrap();
    let titles: Vec<Vec<&str>> = authors
        .iter()
        .map(|author| {
            author
                .posts
                .loaded()
                .unwrap()
                .iter()
                .map(|post| post.title.as_str())
                .collect()
        })
        .collect();
    assert_eq!(titles, [["Ada 3", "Ada 2"], ["Grace 3", "Grace 2"]]);
}

#[tokio::test]
async fn memory_loads_page_each_parent() {
    loads_page_each_parent(Memory::new()).await;
}

#[tokio::test]
async fn sqlite_loads_page_each_parent() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Author::DEF, &Post::DEF]).await.unwrap();
    loads_page_each_parent(sqlite).await;
}
