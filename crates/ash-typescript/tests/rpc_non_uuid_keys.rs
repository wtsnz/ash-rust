//! Records with a primary key of another type over RPC, as AshTypescript takes them: an
//! integer key given as the number it is, a text key as itself, each typed so in the
//! generated client.

use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};

pub mod tally {
    use super::*;

    resource! {
        Tally {
            table "rpc_tallies";

            attributes {
                id: i64 [pk];
                label: String;
            }

            actions {
                create create { primary; accept [label]; }
                read read { primary; }
                update relabel { primary; accept [label]; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod country {
    use super::*;

    resource! {
        Country {
            table "rpc_countries";

            attributes {
                code: String [pk];
                name: String;
            }

            actions {
                create create { primary; accept [code, name]; }
                read read { primary; }
                update rename { primary; accept [name]; }
            }
        }
    }
}

use country::Country;
use tally::Tally;

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Tally>("create_tally", "create")
        .action::<Tally>("relabel_tally", "relabel")
        .action::<Tally>("destroy_tally", "destroy")
        .action::<Tally>("list_tallies", "read")
        .action::<Country>("create_country", "create")
        .action::<Country>("rename_country", "rename")
}

async fn run(ctx: &Context<Memory>, request: Json) -> Json {
    rpc().run(ctx, &request).await
}

#[tokio::test]
async fn an_integer_key_is_given_as_a_number() {
    let ctx = Context::new(Memory::new());
    let created = run(&ctx, json!({ "action": "create_tally", "input": { "label": "a" }, "fields": ["id", "label"] })).await;
    assert_eq!(created, json!({ "success": true, "data": { "id": 1, "label": "a" } }));

    let relabelled =
        run(&ctx, json!({ "action": "relabel_tally", "identity": 1, "input": { "label": "b" }, "fields": ["id", "label"] })).await;
    assert_eq!(relabelled, json!({ "success": true, "data": { "id": 1, "label": "b" } }));

    let missing = run(&ctx, json!({ "action": "relabel_tally", "identity": 2, "input": { "label": "c" } })).await;
    assert_eq!(missing["errors"][0]["type"], "not_found", "{missing}");
    let invalid = run(&ctx, json!({ "action": "relabel_tally", "identity": "one", "input": { "label": "c" } })).await;
    assert_eq!(invalid["success"], false, "{invalid}");

    let destroyed = run(&ctx, json!({ "action": "destroy_tally", "identity": 1, "fields": ["label"] })).await;
    assert_eq!(destroyed, json!({ "success": true, "data": { "label": "b" } }));
    let listed = run(&ctx, json!({ "action": "list_tallies", "fields": ["id"] })).await;
    assert_eq!(listed["data"], json!([]));
}

#[tokio::test]
async fn a_text_key_is_given_as_itself() {
    let ctx = Context::new(Memory::new());
    let created = run(
        &ctx,
        json!({ "action": "create_country", "input": { "code": "NZ", "name": "New Zealand" }, "fields": ["code", "name"] }),
    )
    .await;
    assert_eq!(created["data"], json!({ "code": "NZ", "name": "New Zealand" }), "{created}");
    let renamed =
        run(&ctx, json!({ "action": "rename_country", "identity": "NZ", "input": { "name": "Aotearoa" }, "fields": ["name"] })).await;
    assert_eq!(renamed, json!({ "success": true, "data": { "name": "Aotearoa" } }));
}

#[test]
fn the_client_types_each_key_as_it_is() {
    let client = rpc().typescript_client(&Default::default());
    assert!(client.rpc.contains("identity: number;"), "{}", client.rpc);
    assert!(client.rpc.contains("identity: string;"), "{}", client.rpc);
}
