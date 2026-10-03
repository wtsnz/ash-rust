//! AshTypescript's RPC run on ash-rust: what each kind of action answers, in
//! `AshTypescript.Rpc.run_action`'s shape, and the errors it fails with.

use ash_core::{Actor, Context, Value};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

mod writer {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Writer {
            table "writers";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            actions {
                read read { primary; }
                create create { primary; accept [id, name]; }
            }
        }
    }
}

mod post {
    use ash_core::{FieldMap, resource};
    use uuid::Uuid;

    use super::note::Note;
    use super::writer::Writer;

    resource! {
        Post {
            table "posts";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                title: String;
                draft: bool [default: false];
                score: i64 [default: 0];
                writer_id: Option<Uuid>;
                /// Only editors, and the post's writer, read it.
                editor_note: Option<String>;
            }

            relationships {
                belongs_to writer: Writer [fk: writer_id];
                has_many notes: Note [fk: post_id];
            }

            aggregates {
                note_count: Option<i64> = count(notes);
            }

            calculations {
                doubled: i64 = score * 2;
            }

            policies {
                policy action_type(read) {
                    authorize_if actor_attribute_equals(role, "editor");
                    authorize_if eq(draft, false);
                }

                policy action_type(create) {
                    authorize_if actor_attribute_equals(role, "editor");
                }

                policy action_type(update) {
                    authorize_if actor_attribute_equals(role, "editor");
                }

                policy action_type(destroy) {
                    authorize_if actor_attribute_equals(role, "editor");
                }
            }

            field_policies {
                field editor_note {
                    authorize_if actor_attribute_equals(role, "editor");
                    authorize_if relates_to_actor(writer_id);
                }
            }

            actions {
                read read { primary; }

                create publish {
                    primary;
                    accept [id, title, draft, score, writer_id, editor_note];
                    argument notes: Option<Vec<FieldMap>>;
                    validate string_length(title, min: 3);
                    change manage_relationship(notes, create);
                }

                update rescore {
                    accept [score];
                }

                destroy remove { primary; }
            }
        }
    }
}

mod note {
    use ash_core::resource;
    use uuid::Uuid;

    use super::post::Post;

    resource! {
        Note {
            table "notes";

            attributes {
                id: Uuid [pk];
                post_id: Uuid;
                body: String;
            }

            relationships {
                belongs_to post: Post [fk: post_id];
            }

            actions {
                read read { primary; }
                create create { primary; accept [post_id, body]; }
            }
        }
    }
}

use note::Note;
use post::Post;
use writer::Writer;

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Post>("list_posts", "read")
        .get_by::<Post>("get_post", "read", &["id"])
        .action::<Post>("publish_post", "publish")
        .action::<Post>("rescore_post", "rescore")
        .action::<Post>("remove_post", "remove")
}

struct Blog {
    ctx: Context<Memory>,
    rpc: Rpc<Memory>,
    writer: Uuid,
    published: Vec<Uuid>,
    draft: Uuid,
}

impl Blog {
    /// Three published posts by one writer, scored 1 to 3, the first two with an editor's
    /// note, the second with two notes, and a draft only an editor reads.
    async fn new() -> Self {
        let ctx = Context::new(Memory::new());
        let writer = Writer::create(&ctx).id(Uuid::new_v4()).name("Ada").await.unwrap();
        let editor = ctx.with_actor(editor());
        let mut published = Vec::new();
        for score in 1..=3 {
            let note = match score {
                1 => Some("cut"),
                2 => Some("keep"),
                _ => None,
            };
            let post = Post::publish(&editor)
                .id(Uuid::new_v4())
                .title(format!("Post {score}"))
                .score(score)
                .writer_id(Some(writer.id))
                .editor_note(note.map(str::to_string))
                .await
                .unwrap();
            published.push(post.id);
        }
        for body in ["first", "second"] {
            Note::create(&ctx).post_id(published[1]).body(body).await.unwrap();
        }
        let draft = Post::publish(&editor).id(Uuid::new_v4()).title("Unfinished").draft(true).await.unwrap().id;
        Self { ctx, rpc: rpc(), writer: writer.id, published, draft }
    }

    async fn run(&self, actor: Actor, request: Json) -> Json {
        self.rpc.run(&self.ctx.with_actor(actor), &request).await
    }
}

fn editor() -> Actor {
    Actor::new(Uuid::new_v4()).with_attr("role", Value::from("editor"))
}

fn reader() -> Actor {
    Actor::new(Uuid::new_v4()).with_attr("role", Value::from("reader"))
}

fn error_types(response: &Json) -> Vec<&str> {
    assert_eq!(response["success"], false, "{response}");
    response["errors"].as_array().unwrap().iter().map(|e| e["type"].as_str().unwrap()).collect()
}

#[tokio::test]
async fn a_read_answers_the_selected_fields_filtered_and_sorted() {
    let blog = Blog::new().await;
    let answer = blog
        .run(
            reader(),
            json!({
                "action": "list_posts",
                "fields": ["title", "noteCount", "doubled", { "writer": ["name"] }],
                "filter": { "score": { "greaterThanOrEqual": 2 } },
                "sort": "-score",
            }),
        )
        .await;
    assert_eq!(
        answer,
        json!({ "success": true, "data": [
            { "title": "Post 3", "noteCount": 0, "doubled": 6, "writer": { "name": "Ada" } },
            { "title": "Post 2", "noteCount": 2, "doubled": 4, "writer": { "name": "Ada" } },
        ] })
    );
}

#[tokio::test]
async fn read_policies_filter_what_a_read_answers() {
    let blog = Blog::new().await;
    let request = json!({ "action": "list_posts", "fields": ["title"], "filter": { "draft": { "eq": true } } });
    assert_eq!(blog.run(reader(), request.clone()).await["data"], json!([]));
    assert_eq!(blog.run(editor(), request).await["data"], json!([{ "title": "Unfinished" }]));
}

#[tokio::test]
async fn a_relationship_takes_its_own_filter_sort_and_limit() {
    let blog = Blog::new().await;
    let answer = blog
        .run(
            reader(),
            json!({
                "action": "get_post",
                "getBy": { "id": blog.published[1] },
                "fields": ["title", { "notes": { "fields": ["body"], "sort": "-body", "limit": 1 } }],
            }),
        )
        .await;
    assert_eq!(answer["data"], json!({ "title": "Post 2", "notes": [{ "body": "second" }] }));
}

#[tokio::test]
async fn pages_are_offset_or_keyset() {
    let blog = Blog::new().await;
    let offset = blog
        .run(reader(), json!({ "action": "list_posts", "fields": ["title"], "sort": "score", "page": { "limit": 2, "offset": 1, "count": true } }))
        .await;
    assert_eq!(
        offset["data"],
        json!({ "results": [{ "title": "Post 2" }, { "title": "Post 3" }], "hasMore": false, "limit": 2, "offset": 1, "count": 3, "type": "offset" })
    );

    let first = blog.run(reader(), json!({ "action": "list_posts", "fields": ["title"], "sort": "score", "page": { "limit": 2 } })).await;
    assert_eq!(first["data"]["type"], "keyset");
    assert_eq!(first["data"]["hasMore"], true);
    assert_eq!(first["data"]["results"], json!([{ "title": "Post 1" }, { "title": "Post 2" }]));
    let after = first["data"]["nextPage"].clone();
    let second = blog
        .run(reader(), json!({ "action": "list_posts", "fields": ["title"], "sort": "score", "page": { "limit": 2, "after": after } }))
        .await;
    assert_eq!(second["data"]["results"], json!([{ "title": "Post 3" }]));
    assert_eq!(second["data"]["hasMore"], false);
}

#[tokio::test]
async fn a_get_answers_one_record_or_not_found() {
    let blog = Blog::new().await;
    let found = blog.run(reader(), json!({ "action": "get_post", "getBy": { "id": blog.published[0] }, "fields": ["title"] })).await;
    assert_eq!(found, json!({ "success": true, "data": { "title": "Post 1" } }));
    // A draft, which a reader can't read, isn't found.
    let hidden = blog.run(reader(), json!({ "action": "get_post", "getBy": { "id": blog.draft }, "fields": ["title"] })).await;
    assert_eq!(error_types(&hidden), ["not_found"]);
}

#[tokio::test]
async fn a_create_answers_what_it_selects_with_its_list_argument() {
    let blog = Blog::new().await;
    let created = blog
        .run(
            editor(),
            json!({
                "action": "publish_post",
                "input": { "title": "Fresh", "score": 5, "notes": [{ "body": "hello" }] },
                "fields": ["title", "doubled", "noteCount", { "notes": ["body"] }],
            }),
        )
        .await;
    assert_eq!(
        created,
        json!({ "success": true, "data": { "title": "Fresh", "doubled": 10, "noteCount": 1, "notes": [{ "body": "hello" }] } })
    );
    // Selecting nothing answers an empty object.
    let quiet = blog.run(editor(), json!({ "action": "publish_post", "input": { "title": "Quiet" } })).await;
    assert_eq!(quiet, json!({ "success": true, "data": {} }));
}

#[tokio::test]
async fn an_update_finds_its_record_through_the_read() {
    let blog = Blog::new().await;
    let rescore = |id: Uuid| json!({ "action": "rescore_post", "identity": id, "input": { "score": 9 }, "fields": ["score", "doubled"] });
    let updated = blog.run(editor(), rescore(blog.published[0])).await;
    assert_eq!(updated, json!({ "success": true, "data": { "score": 9, "doubled": 18 } }));
    // A reader may read a published post but not update it; a draft it doesn't find.
    assert_eq!(error_types(&blog.run(reader(), rescore(blog.published[0])).await), ["forbidden"]);
    assert_eq!(error_types(&blog.run(reader(), rescore(blog.draft)).await), ["not_found"]);
}

#[tokio::test]
async fn a_destroy_answers_what_it_selects() {
    let blog = Blog::new().await;
    let removed = blog.run(editor(), json!({ "action": "remove_post", "identity": blog.published[2], "fields": ["title"] })).await;
    assert_eq!(removed, json!({ "success": true, "data": { "title": "Post 3" } }));
    let again = blog.run(editor(), json!({ "action": "remove_post", "identity": blog.published[2] })).await;
    assert_eq!(error_types(&again), ["not_found"]);
}

#[tokio::test]
async fn bad_requests_fail_as_ash_typescript_fails_them() {
    let blog = Blog::new().await;
    let cases = [
        (json!({ "action": "nope", "fields": ["title"] }), "action_not_found"),
        (json!({ "action": "list_posts", "fields": ["title", "nope"] }), "unknown_field"),
        (json!({ "action": "list_posts", "fields": ["title", "writer"] }), "requires_field_selection"),
        (json!({ "action": "publish_post", "input": { "title": "x" } }), "invalid_attribute"),
    ];
    for (request, expected) in cases {
        let answer = blog.run(editor(), request.clone()).await;
        assert_eq!(error_types(&answer), [expected], "{request}");
    }
}

#[tokio::test]
async fn filters_and_sorts_read_hidden_fields_as_null() {
    let blog = Blog::new().await;
    let filtered = json!({ "action": "list_posts", "fields": ["title"], "filter": { "editorNote": { "eq": "cut" } } });
    // A reader can't find posts by a note it can't read, nor order them by it.
    assert_eq!(blog.run(reader(), filtered.clone()).await["data"], json!([]));
    assert_eq!(blog.run(editor(), filtered).await["data"], json!([{ "title": "Post 1" }]));

    let sorted = json!({ "action": "list_posts", "fields": ["title", "editorNote"], "sort": "-editorNote,score", "filter": { "draft": { "eq": false } } });
    assert_eq!(
        blog.run(reader(), sorted.clone()).await["data"],
        json!([
            { "title": "Post 1", "editorNote": null },
            { "title": "Post 2", "editorNote": null },
            { "title": "Post 3", "editorNote": null },
        ])
    );
    // Nulls first descending.
    assert_eq!(
        blog.run(editor(), sorted).await["data"],
        json!([
            { "title": "Post 3", "editorNote": null },
            { "title": "Post 2", "editorNote": "keep" },
            { "title": "Post 1", "editorNote": "cut" },
        ])
    );
}

// A field policy checks fields of the record the client didn't select: the writer reads
// its own notes, selecting nothing but them.
#[tokio::test]
async fn field_policies_check_fields_not_selected() {
    let blog = Blog::new().await;
    let writer = Actor::new(blog.writer).with_attr("role", Value::from("reader"));
    let answer = blog.run(writer, json!({ "action": "list_posts", "fields": ["editorNote"], "sort": "score" })).await;
    assert_eq!(answer["data"], json!([{ "editorNote": "cut" }, { "editorNote": "keep" }, { "editorNote": null }]));
}
