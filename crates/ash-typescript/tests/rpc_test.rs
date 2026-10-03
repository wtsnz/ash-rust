//! AshTypescript's RPC run on ash-rust: what each kind of action answers, in
//! `AshTypescript.Rpc.run_action`'s shape, and the errors it fails with.

use ash_core::{Actor, Context, Value};
use ash_memory::Memory;
use ash_typescript::rpc::{Identity, Rpc};
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
                boosted(by: i64): i64 = score + arg(by);
            }

            identities {
                identity by_title: [title];
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

                policy action(summarize) {
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
                read read {
                    primary;
                    pagination keyset: true, countable: true, required: false;
                }

                create publish {
                    primary;
                    accept [id, title, draft, score, writer_id, editor_note];
                    argument notes: Option<Vec<FieldMap>>;
                    validate string_length(title, min: 3);
                    change manage_relationship(notes, create);
                }

                read recent {
                    pagination keyset: true, default_limit: 2, max_page_size: 2;
                }

                update rescore {
                    accept [score];
                }

                destroy remove { primary; }

                generic summarize {
                    argument word: String;
                    returns String;
                }
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
                read read {
                    primary;
                    pagination keyset: true, countable: true, required: false;
                }
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
        .action::<Post>("recent_posts", "recent")
        .action_with::<Post>("rescore_by_title", "rescore", |o| o.identities(vec![Identity::Named("by_title")]))
        .action_with::<Post>("find_post", "read", |o| o.get_by(&["title"]).not_found_error(false))
        .action_with::<Post>("browse_posts", "read", |o| o.enable_filter(false).denied_loads(&["notes"]))
        .generic::<Post, _, _>("summarize", "summarize", |_, input| async move {
            Ok(Value::from(format!("summary: {}", input.get("word").and_then(Value::as_str).unwrap_or_default())))
        })
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
    let removed = blog
        .run(editor(), json!({ "action": "remove_post", "identity": blog.published[2], "fields": ["title", "doubled"] }))
        .await;
    assert_eq!(removed, json!({ "success": true, "data": { "title": "Post 3", "doubled": 6 } }));
    // Nothing left to destroy: a bulk destroy of no records, which succeeds.
    let again = blog.run(editor(), json!({ "action": "remove_post", "identity": blog.published[2] })).await;
    assert_eq!(again, json!({ "success": true, "data": {} }));
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

#[tokio::test]
async fn requests_are_checked_as_ash_typescript_checks_them() {
    let blog = Blog::new().await;
    let id = blog.published[0];
    let cases = [
        (json!({ "action": "list_posts" }), "missing_required_parameter"),
        (json!({ "action": "list_posts", "fields": [] }), "empty_fields_array"),
        (json!({ "action": "list_posts", "fields": "title" }), "invalid_fields_type"),
        (json!({ "action": "list_posts", "fields": ["title", "title"] }), "duplicate_field"),
        (json!({ "action": "list_posts", "fields": ["title"], "input": null }), "invalid_input_format"),
        (json!({ "action": "list_posts", "fields": ["title"], "page": { "size": 2 } }), "invalid_pagination"),
        (json!({ "action": "list_posts", "fields": ["title"], "page": 2 }), "invalid_pagination"),
        (json!({ "action": "get_post", "fields": ["title"] }), "missing_required_input"),
        (json!({ "action": "get_post", "fields": ["title"], "getBy": { "id": id, "title": "x" } }), "unexpected_get_by_fields"),
        (json!({ "action": "get_post", "fields": ["title"], "getBy": { "id": { "eq": id } } }), "invalid_get_by"),
        (json!({ "action": "get_post", "fields": ["title"], "getBy": { "id": id }, "filter": { "score": { "eq": 1 } } }), "filter_not_supported"),
        (json!({ "action": "rescore_post", "identity": id, "sort": "score" }), "sort_not_supported"),
        (json!({ "action": "rescore_post", "identity": id, "page": { "limit": 1 } }), "pagination_not_supported"),
        (json!({ "action": "rescore_post", "input": { "score": 1 } }), "missing_identity"),
        (json!({ "action": "list_posts", "fields": [{ "title": ["x"] }] }), "field_does_not_support_nesting"),
        (json!({ "action": "list_posts", "fields": [{ "noteCount": ["x"] }] }), "invalid_field_selection"),
        (json!({ "action": "list_posts", "fields": [{ "writer": { "fields": ["name"], "limit": 1 } }] }), "invalid_query_opts"),
        (json!({ "action": "list_posts", "fields": [{ "notes": { "fields": ["body"], "page": { "limit": 1 }, "limit": 1 } }] }), "invalid_query_opts"),
        (json!({ "action": "list_posts", "fields": [{ "notes": [] }] }), "requires_field_selection"),
    ];
    for (request, expected) in cases {
        let answer = blog.run(editor(), request.clone()).await;
        assert_eq!(error_types(&answer), [expected], "{request}");
    }
}

#[tokio::test]
async fn pages_follow_the_actions_pagination() {
    let blog = Blog::new().await;
    // A read that must page pages with its default limit unasked, and never more than
    // its largest page.
    let unasked = blog.run(editor(), json!({ "action": "recent_posts", "fields": ["title"], "sort": "score" })).await;
    assert_eq!(unasked["data"]["type"], "keyset");
    assert_eq!(unasked["data"]["limit"], 2);
    let large = blog.run(editor(), json!({ "action": "recent_posts", "fields": ["title"], "sort": "score", "page": { "limit": 10 } })).await;
    assert_eq!(large["data"]["results"].as_array().unwrap().len(), 2);
    // Counting a read that can't be counted fails.
    let counted = blog.run(editor(), json!({ "action": "recent_posts", "fields": ["title"], "page": { "limit": 1, "count": true } })).await;
    assert_eq!(error_types(&counted), ["invalid_page"]);
    // A cursor that isn't one fails.
    let garbage = blog.run(editor(), json!({ "action": "list_posts", "fields": ["title"], "page": { "limit": 1, "after": "garbage" } })).await;
    assert_eq!(error_types(&garbage), ["invalid_keyset"]);
    // One that needn't page, unasked, doesn't.
    let list = blog.run(reader(), json!({ "action": "list_posts", "fields": ["title"], "sort": "score" })).await;
    assert!(list["data"].is_array(), "{list}");
}

#[tokio::test]
async fn calculations_take_arguments() {
    let blog = Blog::new().await;
    let boosted = blog
        .run(reader(), json!({ "action": "list_posts", "fields": ["title", { "boosted": { "args": { "by": 10 } } }], "sort": "score" }))
        .await;
    assert_eq!(boosted["data"], json!([{ "title": "Post 1", "boosted": 11 }, { "title": "Post 2", "boosted": 12 }, { "title": "Post 3", "boosted": 13 }]));
    let bare = blog.run(reader(), json!({ "action": "list_posts", "fields": ["boosted"] })).await;
    assert_eq!(error_types(&bare), ["invalid_field_format"]);
    let missing = blog.run(reader(), json!({ "action": "list_posts", "fields": [{ "boosted": { "args": {} } }] })).await;
    assert_eq!(error_types(&missing), ["invalid_calculation_args"]);
    let extra = blog.run(reader(), json!({ "action": "list_posts", "fields": [{ "doubled": { "args": { "by": 1 } } }] })).await;
    assert_eq!(error_types(&extra), ["invalid_calculation_args"]);
}

#[tokio::test]
async fn relationships_come_as_pages() {
    let blog = Blog::new().await;
    let request = |page: Json| {
        json!({
            "action": "get_post", "getBy": { "id": blog.published[1] },
            "fields": ["title", { "notes": { "fields": ["body"], "sort": "body", "page": page } }],
        })
    };
    let first = blog.run(reader(), request(json!({ "limit": 1, "count": true }))).await;
    let notes = &first["data"]["notes"];
    assert_eq!(notes["type"], "keyset");
    assert_eq!(notes["results"], json!([{ "body": "first" }]));
    assert_eq!((notes["hasMore"].clone(), notes["count"].clone()), (json!(true), json!(2)));
    let next = blog.run(reader(), request(json!({ "limit": 1, "after": notes["nextPage"] }))).await;
    assert_eq!(next["data"]["notes"]["results"], json!([{ "body": "second" }]));
    assert_eq!(next["data"]["notes"]["hasMore"], false);
    // A keyset read takes no offset.
    let offset = blog.run(reader(), request(json!({ "limit": 1, "offset": 1 }))).await;
    assert_eq!(error_types(&offset), ["invalid_pagination"]);
}

#[tokio::test]
async fn updates_name_their_record_by_an_identity() {
    let blog = Blog::new().await;
    let rescored = blog
        .run(editor(), json!({ "action": "rescore_by_title", "identity": { "title": "Post 2" }, "input": { "score": 7 }, "fields": ["title", "score"] }))
        .await;
    assert_eq!(rescored["data"], json!({ "title": "Post 2", "score": 7 }));
    let unknown = blog.run(editor(), json!({ "action": "rescore_by_title", "identity": { "score": 1 }, "input": { "score": 7 } })).await;
    assert_eq!(error_types(&unknown), ["invalid_identity"]);
    let missing = blog.run(editor(), json!({ "action": "rescore_by_title", "identity": { "title": "Nope" }, "input": { "score": 7 } })).await;
    assert_eq!(error_types(&missing), ["not_found"]);
}

#[tokio::test]
async fn actions_take_ash_typescripts_options() {
    let blog = Blog::new().await;
    // `not_found_error?: false`: a get that finds nothing answers null.
    let none = blog.run(reader(), json!({ "action": "find_post", "getBy": { "title": "Nope" }, "fields": ["title"] })).await;
    assert_eq!(none, json!({ "success": true, "data": null }));
    let found = blog.run(reader(), json!({ "action": "find_post", "getBy": { "title": "Post 3" }, "fields": ["score"] })).await;
    assert_eq!(found["data"], json!({ "score": 3 }));
    // `enable_filter?: false` and `denied_loads`.
    let filtered = blog.run(reader(), json!({ "action": "browse_posts", "fields": ["title"], "filter": { "score": { "eq": 1 } } })).await;
    assert_eq!(error_types(&filtered), ["filter_not_supported"]);
    let denied = blog.run(reader(), json!({ "action": "browse_posts", "fields": ["title", { "notes": ["body"] }] })).await;
    assert_eq!(error_types(&denied), ["load_denied"]);
}

#[tokio::test]
async fn generic_actions_are_authorized() {
    let blog = Blog::new().await;
    let request = json!({ "action": "summarize", "input": { "word": "hello" } });
    assert_eq!(error_types(&blog.run(reader(), request.clone()).await), ["forbidden"]);
    assert_eq!(blog.run(editor(), request).await, json!({ "success": true, "data": "summary: hello" }));
    let missing = blog.run(editor(), json!({ "action": "summarize", "input": {} })).await;
    assert_eq!(error_types(&missing), ["required"]);
}

#[tokio::test]
async fn requests_validate_without_running() {
    let blog = Blog::new().await;
    let validate = |actor: Actor, request: Json| {
        let blog = &blog;
        async move { blog.rpc.validate(&blog.ctx.with_actor(actor), &request).await }
    };
    // Every problem, at its field, its message filled in.
    let invalid = validate(editor(), json!({ "action": "publish_post", "input": { "title": "x", "score": "high" } })).await;
    let mut problems: Vec<(String, String)> = invalid["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["fields"][0].as_str().unwrap().to_string(), e["message"].as_str().unwrap().to_string()))
        .collect();
    problems.sort();
    assert_eq!(problems, [("score".into(), "is invalid".into()), ("title".into(), "must have length of at least 3".into())]);
    assert_eq!(invalid["errors"][0]["path"], json!([invalid["errors"][0]["fields"][0]]));
    // Nothing written, nor authorized: a reader validates a publish.
    let valid = validate(reader(), json!({ "action": "publish_post", "input": { "title": "Fine" } })).await;
    assert_eq!(valid, json!({ "success": true }));
    assert_eq!(blog.run(editor(), json!({ "action": "list_posts", "fields": ["title"], "filter": { "title": { "eq": "Fine" } } })).await["data"], json!([]));
    // An update's record is found first.
    let missing = validate(editor(), json!({ "action": "rescore_post", "identity": Uuid::new_v4(), "input": { "score": 1 } })).await;
    assert_eq!(error_types(&missing), ["not_found"]);
    let found = validate(editor(), json!({ "action": "rescore_post", "identity": blog.published[0], "input": { "score": "x" } })).await;
    assert_eq!(found["errors"][0]["message"], "is invalid");
    // A read needs no fields to validate, but its request is checked as a run's.
    assert_eq!(validate(reader(), json!({ "action": "list_posts" })).await, json!({ "success": true }));
    assert_eq!(error_types(&validate(reader(), json!({ "action": "nope" })).await), ["action_not_found"]);
}

#[tokio::test]
async fn errors_pass_through_the_error_handler() {
    let blog = Blog::new().await;
    let rpc = rpc()
        .error_handler(|mut failure, source| {
            if failure.kind == "forbidden" {
                return None;
            }
            failure.message = format!("{}: {}", source.rpc_action.as_deref().unwrap_or("?"), failure.message);
            Some(failure)
        })
        .show_raised_errors(true);
    let ctx = blog.ctx.with_actor(reader());
    let unknown = rpc.run(&ctx, &json!({ "action": "list_posts", "fields": ["nope"] })).await;
    assert_eq!(unknown["errors"][0]["message"], "list_posts: Unknown field %{field} for resource %{resource}");
    let forbidden = rpc.run(&ctx, &json!({ "action": "rescore_post", "identity": blog.published[0], "input": { "score": 1 } })).await;
    assert_eq!(forbidden, json!({ "success": false, "errors": [] }));
}

#[tokio::test]
async fn booleans_cast_from_text_as_ash_casts_them() {
    let blog = Blog::new().await;
    let request = |draft: Json| json!({ "action": "list_posts", "fields": ["title"], "filter": { "draft": { "eq": draft } } });
    assert_eq!(blog.run(editor(), request(json!("true"))).await["data"], json!([{ "title": "Unfinished" }]));
    let bad = blog.run(editor(), request(json!("yes"))).await;
    assert_eq!(bad["success"], false);
}
