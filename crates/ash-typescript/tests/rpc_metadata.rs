//! Action metadata over RPC, as AshTypescript shows it: what an action notes on the
//! records it answers (`metadata :name, :type`, set with `put_metadata`), merged into a
//! read's records when `metadataFields` asks for it, and beside a write's data by
//! default, as far as `show_metadata` exposes it, under `metadata_field_names`.

use ash_core::{ChangeContext, Context, FieldMap, Value, put_metadata, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

resource! {
    Item {
        table "rpc_metadata_items";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        actions {
            read read { primary; }

            read ranked {
                metadata rank: i64;
                metadata note: Option<String>;
                prepare sort(name);
                prepare after_action(rank_items);
            }

            create create {
                primary;
                accept [name];
                argument tags: Vec<String> [default: Vec::new()];
                metadata tag_count: i64;
                metadata echo: String;
                change func(note_tags);
            }
        }
    }
}

fn rank_items(_arguments: &FieldMap, records: &mut [FieldMap]) -> ash_core::Result<()> {
    for (rank, record) in records.iter_mut().enumerate() {
        put_metadata(record, "rank", rank as i64 + 1);
    }
    Ok(())
}

fn note_tags(ctx: &mut ChangeContext<'_>) -> ash_core::Result<()> {
    let count = ctx.arguments.get("tags").and_then(Value::as_array).map_or(0, <[Value]>::len) as i64;
    ctx.after_action(move |record| {
        put_metadata(record, "tag_count", count);
        put_metadata(record, "echo", "hi");
        Ok(())
    });
    Ok(())
}

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Item>("list_ranked", "ranked")
        .action::<Item>("create_item", "create")
        .action_with::<Item>("create_quietly", "create", |options| options.show_metadata(&["tag_count"]))
        .action_with::<Item>("create_renamed", "create", |options| options.metadata_field_names(&[("tag_count", "tags_given")]))
}

async fn run(ctx: &Context<Memory>, request: Json) -> Json {
    rpc().run(ctx, &request).await
}

#[tokio::test]
async fn a_write_shows_its_metadata_beside_its_data() {
    let ctx = Context::new(Memory::new());
    let created = run(&ctx, json!({ "action": "create_item", "input": { "name": "a", "tags": ["x", "y"] }, "fields": ["name"] })).await;
    assert_eq!(created, json!({ "success": true, "data": { "name": "a" }, "metadata": { "tagCount": 2, "echo": "hi" } }));

    // Only what it asks for, and only what's exposed.
    let asked = run(&ctx, json!({ "action": "create_item", "input": { "name": "b" }, "metadataFields": ["echo"] })).await;
    assert_eq!(asked, json!({ "success": true, "data": {}, "metadata": { "echo": "hi" } }));
    let quiet = run(&ctx, json!({ "action": "create_quietly", "input": { "name": "c" }, "fields": ["name"], "metadataFields": ["echo", "tagCount"] })).await;
    assert_eq!(quiet["metadata"], json!({ "tagCount": 0 }));

    // Under the name the client knows it by.
    let renamed = run(&ctx, json!({ "action": "create_renamed", "input": { "name": "d", "tags": ["x"] }, "fields": ["name"] })).await;
    assert_eq!(renamed["metadata"], json!({ "tags_given": 1, "echo": "hi" }));
}

#[tokio::test]
async fn a_read_merges_the_metadata_asked_for_into_its_records() {
    let ctx = Context::new(Memory::new());
    for name in ["b", "a", "c"] {
        run(&ctx, json!({ "action": "create_item", "input": { "name": name } })).await;
    }
    // Unasked, a read shows none.
    let plain = run(&ctx, json!({ "action": "list_ranked", "fields": ["name"] })).await;
    assert_eq!(plain["data"], json!([{ "name": "a" }, { "name": "b" }, { "name": "c" }]));

    let ranked = run(&ctx, json!({ "action": "list_ranked", "fields": ["name"], "metadataFields": ["rank", "note"] })).await;
    assert_eq!(
        ranked["data"],
        json!([
            { "name": "a", "rank": 1, "note": null },
            { "name": "b", "rank": 2, "note": null },
            { "name": "c", "rank": 3, "note": null },
        ])
    );
}

#[test]
#[should_panic(expected = "which `create` doesn't declare")]
fn showing_undeclared_metadata_is_refused() {
    let _ = Rpc::<Memory>::new().action_with::<Item>("bad", "create", |options| options.show_metadata(&["nope"]));
}

#[test]
fn actions_declare_their_metadata() {
    use ash_core::{AttrType, Resource};
    let create = Item::DEF.action("create").unwrap();
    assert_eq!(create.metadata.len(), 2);
    assert_eq!((create.metadata[0].name, create.metadata[0].ty, create.metadata[0].allow_nil), ("tag_count", AttrType::Integer, false));
    let ranked = Item::DEF.action("ranked").unwrap();
    assert!(ranked.metadata[1].allow_nil);
}
