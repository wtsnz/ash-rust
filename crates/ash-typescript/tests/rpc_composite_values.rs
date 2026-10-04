//! Selecting within an embedded resource, a typed map or a union over RPC, as
//! AshTypescript selects within them: each needs what it holds selected, comes back with
//! only that (its fields named in camelCase), and a union as `{member: value}`, null when
//! its member isn't selected.

use ash_core::{AshTypedMap, AshUnion, Context, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

pub mod address {
    use ash_core::resource;

    resource! {
        embedded Address {
            attributes {
                street: String;
                zip_code: String;
                note: Option<String>;
            }

            actions {
                create create { primary; accept [street, zip_code, note]; }
            }
        }
    }
}

use address::Address;

#[derive(Clone, Debug, PartialEq, AshTypedMap)]
pub struct Size {
    pub width: i64,
    pub height: i64,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, AshUnion)]
pub enum Content {
    Text(String),
    Sized(Size),
}

resource! {
    Parcel {
        table "rpc_composite_parcels";

        attributes {
            id: Uuid [pk];
            name: String;
            address: Option<Address>;
            size: Option<Size>;
            content: Option<Content>;
            stops: Vec<Address> [default: Vec::new()];
        }

        actions {
            read read { primary; }
            create create { primary; accept [name, address, size, content, stops]; }
        }
    }
}

fn rpc() -> Rpc<Memory> {
    Rpc::new().action::<Parcel>("list_parcels", "read").action::<Parcel>("create_parcel", "create")
}

async fn run(ctx: &Context<Memory>, request: Json) -> Json {
    rpc().run(ctx, &request).await
}

async fn seeded() -> Context<Memory> {
    let ctx = Context::new(Memory::new());
    let created = run(
        &ctx,
        json!({
            "action": "create_parcel",
            "input": {
                "name": "a",
                "address": { "street": "1 Main St", "zipCode": "94105" },
                "size": { "width": 2, "height": 3 },
                "content": { "sized": { "width": 4, "height": 5, "label": "box" } },
                "stops": [{ "street": "2 Side St", "zipCode": "10001" }],
            },
            "fields": ["name"],
        }),
    )
    .await;
    assert_eq!(created["success"], json!(true), "{created}");
    run(&ctx, json!({ "action": "create_parcel", "input": { "name": "b", "content": { "text": "hello" } } })).await;
    ctx
}

#[tokio::test]
async fn values_come_back_with_only_what_is_selected() {
    let ctx = seeded().await;
    let listed = run(
        &ctx,
        json!({
            "action": "list_parcels",
            "fields": [
                "name",
                { "address": ["zipCode", "note"] },
                { "size": ["width"] },
                { "content": ["text", { "sized": ["label"] }] },
                { "stops": ["street"] },
            ],
        }),
    )
    .await;
    let mut parcels = listed["data"].as_array().unwrap().clone();
    parcels.sort_by_key(|parcel| parcel["name"].as_str().unwrap().to_string());
    assert_eq!(
        parcels,
        [
            json!({
                "name": "a",
                "address": { "zipCode": "94105", "note": null },
                "size": { "width": 2 },
                "content": { "sized": { "label": "box" } },
                "stops": [{ "street": "2 Side St" }],
            }),
            json!({ "name": "b", "address": null, "size": null, "content": { "text": "hello" }, "stops": [] }),
        ]
    );

    // A member not selected is null.
    let text_only = run(&ctx, json!({ "action": "list_parcels", "fields": ["name", { "content": ["text"] }] })).await;
    let a = text_only["data"].as_array().unwrap().iter().find(|parcel| parcel["name"] == "a").unwrap().clone();
    assert_eq!(a["content"], Json::Null);
}

#[tokio::test]
async fn values_need_what_they_hold_selected() {
    let ctx = seeded().await;
    let error = |response: Json| (response["errors"][0]["type"].clone(), response["errors"][0]["vars"].clone());
    for (fields, expected) in [
        (json!(["address"]), json!({ "fieldType": "Embedded_resource", "field": "address" })),
        (json!(["content"]), json!({ "fieldType": "Union_attribute", "field": "content" })),
        (json!(["size"]), json!({ "fieldType": "Field_constrained_type", "field": "size" })),
        (json!([{ "content": ["sized"] }]), json!({ "fieldType": "Complex_type", "field": "content.sized" })),
        (json!([{ "address": [] }]), json!({ "fieldType": "Embedded_resource", "field": "address" })),
    ] {
        let response = run(&ctx, json!({ "action": "list_parcels", "fields": fields })).await;
        assert_eq!(error(response), (json!("requires_field_selection"), expected), "{fields}");
    }
    let unknown = run(&ctx, json!({ "action": "list_parcels", "fields": [{ "address": ["nope"] }] })).await;
    assert_eq!(unknown["errors"][0]["type"], json!("unknown_field"), "{unknown}");
    let not_nested = run(&ctx, json!({ "action": "list_parcels", "fields": [{ "size": [{ "width": ["x"] }] }] })).await;
    assert_eq!(not_nested["errors"][0]["type"], json!("field_does_not_support_nesting"), "{not_nested}");
}

#[tokio::test]
async fn typed_values_round_trip() {
    let ctx = seeded().await;
    let parcels = Parcel::query(&ctx).all().await.unwrap();
    let a = parcels.iter().find(|parcel| parcel.name == "a").unwrap();
    assert_eq!(a.size, Some(Size { width: 2, height: 3, label: None }));
    assert_eq!(a.content, Some(Content::Sized(Size { width: 4, height: 5, label: Some("box".into()) })));
    assert_eq!(a.address.as_ref().map(|address| address.zip_code.as_str()), Some("94105"));
}
