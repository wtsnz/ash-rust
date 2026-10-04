//! The GraphQL client's types and selections for embedded resources, typed maps and
//! unions: their fields in camelCase, a union's members by `__typename`, each selected
//! as ash-graphql serves it, so the client's query runs and its types check.

use std::process::Command;

use ash_core::{AshTypedMap, AshUnion, Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use async_graphql::Request;
use uuid::Uuid;

pub mod address {
    use ash_core::resource;

    resource! {
        embedded Address {
            attributes {
                street: String;
                zip_code: Option<String>;
            }

            actions {
                create create { primary; accept [street, zip_code]; }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, AshTypedMap)]
pub struct Size {
    pub width: i64,
}

#[derive(Clone, Debug, PartialEq, AshUnion)]
pub enum Content {
    Text(String),
    Sized(Size),
}

resource! {
    Parcel {
        table "client_composite_parcels";

        attributes {
            id: Uuid [pk];
            address: Option<address::Address>;
            content: Option<Content>;
        }

        actions {
            read read { primary; pagination keyset: true, required: false; }
            create create { primary; accept [address, content]; }
        }
    }
}

#[tokio::test]
async fn the_client_selects_and_types_composite_values() {
    let ts = TypeScriptGenerator::new()
        .config(TypeScriptConfig::new().with_zod(false).with_client(true).with_react(false).with_client_name("Api"))
        .add_resource(&Parcel::DEF)
        .generate_consolidated()
        .unwrap();
    assert!(ts.contains("address?: { street: string; zipCode?: string | null } | null;"), "{ts}");
    assert!(ts.contains(r#"content?: { __typename: "ContentText"; value: string } | { __typename: "ContentSized"; value: { width: number } } | null;"#), "{ts}");

    // The client's selection runs against the schema.
    let selection = ts.lines().find_map(|line| line.trim().strip_prefix("let fields = \"")).and_then(|rest| rest.strip_suffix("\";")).unwrap();
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&Parcel::DEF]).finish::<Memory>().unwrap();
    let created = schema
        .execute(Request::new(r#"mutation { createParcel(input: { address: { street: "1 Main" }, content: { sized: { width: 2 } } }) { errors { message } } }"#).data(ctx.clone()))
        .await;
    assert!(created.errors.is_empty(), "{:?}", created.errors);
    let listed = schema.execute(Request::new(format!("{{ listParcels {{ results {{ {selection} }} }} }}")).data(ctx)).await;
    assert!(listed.errors.is_empty(), "{selection}: {:?}", listed.errors);
    let parcel = listed.data.into_json().unwrap()["listParcels"]["results"][0].clone();
    assert_eq!(parcel["content"], serde_json::json!({ "__typename": "ContentSized", "value": { "width": 2 } }));

    let dir = std::env::temp_dir().join(format!("ash_ts_composites_{}", Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("client.ts"), &ts).unwrap();
    let Ok(output) = Command::new("tsc").args(["--noEmit", "--target", "ES2022", "--moduleResolution", "node", "--skipLibCheck"]).arg(dir.join("client.ts")).output() else {
        return;
    };
    assert!(output.status.success(), "tsc failed:\n{}", String::from_utf8_lossy(&output.stdout));
}
