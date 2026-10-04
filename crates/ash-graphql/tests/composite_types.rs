//! Embedded resources, typed maps and unions in GraphQL, as AshGraphql types them: an
//! embedded resource its own object type (and an input per attribute holding it), a
//! typed map an object of its fields, a union a GraphQL union of an object per member,
//! each `{ value }`; given as inputs of the same shapes.

use ash_core::{AshTypedMap, AshUnion, Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};
use uuid::Uuid;

pub mod address {
    use ash_core::resource;

    resource! {
        embedded Address {
            attributes {
                street: String;
                zip_code: String;
            }

            actions {
                create create { primary; accept [street, zip_code]; }
            }
        }
    }
}

use address::Address;

#[derive(Clone, Debug, PartialEq, AshTypedMap)]
pub struct Size {
    pub width: i64,
    pub label: Option<String>,
}

#[derive(Clone, Debug, PartialEq, AshUnion)]
pub enum Content {
    Text(String),
    Sized(Size),
}

resource! {
    Parcel {
        table "gql_composite_parcels";

        attributes {
            id: Uuid [pk];
            address: Option<Address>;
            size: Option<Size>;
            content: Option<Content>;
            stops: Vec<Address> [default: Vec::new()];
        }

        actions {
            read read { primary; }
            create create { primary; accept [address, size, content, stops]; }
        }
    }
}

async fn run(ctx: &Context<Memory>, query: &str) -> Value {
    let schema = AshGraphQL::from_resources(&[&Parcel::DEF]).finish::<Memory>().unwrap();
    let response = schema.execute(Request::new(query).data(ctx.clone())).await;
    assert!(response.errors.is_empty(), "{query}: {:?}", response.errors);
    response.data.into_json().unwrap()
}

#[tokio::test]
async fn composite_values_have_their_own_types() {
    let ctx = Context::new(Memory::new());
    let created = run(
        &ctx,
        r#"mutation {
            createParcel(input: {
                address: { street: "1 Main St", zipCode: "94105" },
                size: { width: 3 },
                content: { sized: { width: 4, label: "box" } },
                stops: [{ street: "2 Side St", zipCode: "10001" }]
            }) {
                result {
                    address { street zipCode }
                    size { width label }
                    content {
                        __typename
                        ... on ContentText { value }
                        ... on ContentSized { value { width label } }
                    }
                    stops { zipCode }
                }
                errors { message }
            }
        }"#,
    )
    .await;
    assert_eq!(
        created["createParcel"],
        json!({
            "result": {
                "address": { "street": "1 Main St", "zipCode": "94105" },
                "size": { "width": 3, "label": null },
                "content": { "__typename": "ContentSized", "value": { "width": 4, "label": "box" } },
                "stops": [{ "zipCode": "10001" }],
            },
            "errors": [],
        })
    );
    run(&ctx, r#"mutation { createParcel(input: { content: { text: "hi" } }) { errors { message } } }"#).await;
    let listed = run(&ctx, "{ listParcels { content { ... on ContentText { value } } } }").await;
    let contents: Vec<&Value> = listed["listParcels"].as_array().unwrap().iter().map(|parcel| &parcel["content"]).collect();
    assert!(contents.contains(&&json!({ "value": "hi" })), "{contents:?}");

    let sdl = AshGraphQL::from_resources(&[&Parcel::DEF]).finish::<Memory>().unwrap().sdl();
    for expected in [
        "type Address {",
        "input ParcelAddressInput {",
        "union Content = ContentSized | ContentText",
        "input ContentInput {",
        "type Size {",
        "input SizeInput {",
        "address: Address",
        "stops: [Address!]!",
    ] {
        assert!(sdl.contains(expected), "{expected}\n{sdl}");
    }
}
