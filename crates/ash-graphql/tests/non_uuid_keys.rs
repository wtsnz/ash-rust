//! A primary key of any type is the record's `id: ID!`, as AshGraphql encodes one by
//! default: an integer key given and answered as text (as Absinthe serializes an ID), a
//! text key as itself and as its own field too.

use ash_core::{Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};

pub mod tally {
    use super::*;

    resource! {
        Tally {
            table "gql_tallies";

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
            table "gql_countries";

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

fn schema() -> async_graphql::dynamic::Schema {
    AshGraphQL::from_resources(&[&Tally::DEF, &Country::DEF]).finish::<Memory>().unwrap()
}

async fn run(ctx: &Context<Memory>, query: &str) -> async_graphql::Response {
    schema().execute(Request::new(query).data(ctx.clone())).await
}

async fn data(ctx: &Context<Memory>, query: &str) -> Value {
    let response = run(ctx, query).await;
    assert!(response.errors.is_empty(), "{query}: {:?}", response.errors);
    response.data.into_json().unwrap()
}

#[tokio::test]
async fn an_integer_key_is_an_id() {
    let ctx = Context::new(Memory::new());
    let sdl = schema().sdl();
    assert!(sdl.contains("type Tally {\n\tid: ID!\n\tlabel: String!\n"), "{sdl}");
    assert!(sdl.contains("getTally(id: ID!): Tally"), "{sdl}");

    let created = data(&ctx, r#"mutation { createTally(input: { label: "a" }) { result { id label } errors { code } } }"#).await;
    assert_eq!(created["createTally"]["result"], json!({ "id": "1", "label": "a" }), "{created}");

    // Given as text or as a number.
    assert_eq!(data(&ctx, r#"{ getTally(id: "1") { id label } }"#).await, json!({ "getTally": { "id": "1", "label": "a" } }));
    assert_eq!(data(&ctx, "{ getTally(id: 1) { label } }").await, json!({ "getTally": { "label": "a" } }));
    assert_eq!(data(&ctx, r#"{ getTally(id: "2") { label } }"#).await, json!({ "getTally": null }));

    let relabelled =
        data(&ctx, r#"mutation { relabelTally(id: "1", input: { label: "b" }) { result { id label } errors { code } } }"#).await;
    assert_eq!(relabelled["relabelTally"]["result"], json!({ "id": "1", "label": "b" }));

    let destroyed = data(&ctx, r#"mutation { destroyTally(id: "1") { result { id } errors { code } } }"#).await;
    assert_eq!(destroyed["destroyTally"]["result"], json!({ "id": "1" }));
    assert_eq!(data(&ctx, r#"{ getTally(id: "1") { label } }"#).await, json!({ "getTally": null }));

    // Text that is no integer names no record of an integer key.
    let invalid = run(&ctx, r#"{ getTally(id: "one") { label } }"#).await;
    assert!(!invalid.errors.is_empty(), "{:?}", invalid.data);
}

#[tokio::test]
async fn a_text_key_is_an_id_and_its_own_field() {
    let ctx = Context::new(Memory::new());
    let sdl = schema().sdl();
    assert!(sdl.contains("type Country {\n\tid: ID!\n\tcode: String!\n"), "{sdl}");

    let created = data(
        &ctx,
        r#"mutation { createCountry(input: { code: "NZ", name: "New Zealand" }) { result { id code name } errors { code } } }"#,
    )
    .await;
    assert_eq!(created["createCountry"]["result"], json!({ "id": "NZ", "code": "NZ", "name": "New Zealand" }));

    assert_eq!(data(&ctx, r#"{ getCountry(id: "NZ") { name } }"#).await, json!({ "getCountry": { "name": "New Zealand" } }));
    let renamed =
        data(&ctx, r#"mutation { renameCountry(id: "NZ", input: { name: "Aotearoa" }) { result { name } errors { code } } }"#).await;
    assert_eq!(renamed["renameCountry"]["result"], json!({ "name": "Aotearoa" }));
}
