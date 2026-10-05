//! A map is the `Json` scalar, given and answered as an object. It's also taken as JSON
//! text in a string, as AshGraphql takes a map under either of its JSON scalars, so a
//! client written for AshGraphql works as it is. (AshGraphql answers a map as a string by
//! default, `JsonString`; ash-rust answers an object, as AshGraphql's `json_type: :json`
//! does. GAPS.md row 30 says why.)

use ash_core::{Context, FieldMap, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};
use uuid::Uuid;

resource! {
    Profile {
        table "json_profiles";

        attributes {
            id: Uuid [pk];
            settings: Option<FieldMap>;
            history: Vec<FieldMap> [default: Vec::new()];
        }

        actions {
            create create { primary; accept [settings, history]; }
            read read { primary; }
        }
    }
}

async fn create(input: &str) -> Value {
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&Profile::DEF]).finish::<Memory>().unwrap();
    let query = format!("mutation {{ createProfile(input: {input}) {{ result {{ settings history }} errors {{ code }} }} }}");
    let response = schema.execute(Request::new(query).data(ctx)).await;
    assert!(response.errors.is_empty(), "{:?}", response.errors);
    response.data.into_json().unwrap()["createProfile"].clone()
}

#[tokio::test]
async fn a_map_is_taken_as_an_object_or_as_json_text() {
    let expected = json!({ "settings": { "theme": "dark", "size": 2 }, "history": [{ "at": 1 }, { "at": 2 }] });

    let objects = create(r#"{ settings: { theme: "dark", size: 2 }, history: [{ at: 1 }, { at: 2 }] }"#).await;
    assert_eq!(objects["result"], expected, "{objects}");

    let text = create(r#"{ settings: "{\"theme\":\"dark\",\"size\":2}", history: ["{\"at\":1}", "{\"at\":2}"] }"#).await;
    assert_eq!(text["result"], expected, "{text}");
}

#[tokio::test]
async fn json_text_for_a_map_must_hold_an_object() {
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&Profile::DEF]).finish::<Memory>().unwrap();
    for input in [r#"{ history: ["5"] }"#, r#"{ history: ["[]"] }"#, r#"{ settings: "true" }"#, r#"{ settings: "\"text\"" }"#, r#"{ history: [5] }"#, r#"{ settings: "{" }"#] {
        let query = format!("mutation {{ createProfile(input: {input}) {{ result {{ id }} }} }}");
        let response = schema.execute(Request::new(query).data(ctx.clone())).await;
        assert!(!response.errors.is_empty(), "{input} should be refused");
    }
    let stored = Profile::query(&ctx).all().await;
    assert!(stored.is_ok_and(|rows| rows.is_empty()), "nothing was stored");
}
