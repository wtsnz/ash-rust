//! How clients name fields and arguments over RPC: in camelCase, even where that doesn't
//! survive the trip back (`line_1` is `line1`), or as `field_names` and `argument_names`
//! map them, as AshTypescript's do, in selections, input, filters, sorts, `getBy`, output
//! and errors.

use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

resource! {
    Place {
        table "rpc_names_places";

        attributes {
            id: Uuid [pk];
            line_1: String;
            postcode: Option<String>;
        }

        actions {
            read read { primary; }
            create create {
                primary;
                accept [line_1, postcode];
                argument note_2: Option<String>;
                validate string_length(line_1, min: 2);
                change func(read_note);
            }
        }
    }
}

/// Reads the argument, so a client's name for it is seen to arrive.
fn read_note(ctx: &mut ash_core::ChangeContext<'_>) -> ash_core::Result<()> {
    if let Some(note) = ctx.arguments.get("note_2").filter(|note| !note.is_null()) {
        ctx.fields.insert("postcode".into(), note.clone());
    }
    Ok(())
}

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Place>("list_places", "read")
        .action::<Place>("create_place", "create")
        .get_by::<Place>("get_place", "read", &["postcode"])
        .field_names::<Place>(&[("postcode", "zip")])
        .argument_names::<Place>("create", &[("note_2", "remark")])
}

async fn run(ctx: &Context<Memory>, request: Json) -> Json {
    rpc().run(ctx, &request).await
}

#[tokio::test]
async fn fields_and_arguments_go_by_their_client_names() {
    let ctx = Context::new(Memory::new());
    let created = run(
        &ctx,
        json!({ "action": "create_place", "input": { "line1": "1 Main St", "remark": "94105" }, "fields": ["line1", "zip"] }),
    )
    .await;
    // The argument arrived under its own name, and set the postcode.
    assert_eq!(created, json!({ "success": true, "data": { "line1": "1 Main St", "zip": "94105" } }));
    run(&ctx, json!({ "action": "create_place", "input": { "line1": "2 Side St", "zip": "10001" } })).await;

    let listed = run(
        &ctx,
        json!({ "action": "list_places", "fields": ["zip"], "filter": { "zip": { "eq": "10001" } }, "sort": "-line1" }),
    )
    .await;
    assert_eq!(listed["data"], json!([{ "zip": "10001" }]));
    let got = run(&ctx, json!({ "action": "get_place", "getBy": { "zip": "94105" }, "fields": ["line1"] })).await;
    assert_eq!(got["data"], json!({ "line1": "1 Main St" }));

    // Errors name the field as the client does.
    let invalid = run(&ctx, json!({ "action": "create_place", "input": { "line1": "x" } })).await;
    assert_eq!(invalid["errors"][0]["fields"], json!(["line1"]), "{invalid}");
}
