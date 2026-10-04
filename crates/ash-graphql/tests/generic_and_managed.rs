//! Generic actions and managed relationship inputs, as AshGraphql serves them: a generic
//! action as a query taking its arguments or a mutation taking them as `input`, answering
//! what it returns, or `true`; an argument a `manage_relationship` change reads as an
//! input object of what the destination's actions take.

use ash_core::{Context, Error, FieldMap, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};
use uuid::Uuid;

pub mod line {
    use super::*;

    resource! {
        Line {
            table "gm_lines";

            attributes {
                id: Uuid [pk];
                order_id: Uuid;
                item: String;
                quantity: i64 [default: 1];
            }

            relationships {
                belongs_to order: super::order::Order [fk: order_id];
            }

            actions {
                create create { primary; accept [order_id, item, quantity]; }
                read read { primary; }
                update update { primary; accept [item, quantity]; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod order {
    use super::*;

    resource! {
        Order {
            table "gm_orders";

            attributes {
                id: Uuid [pk];
                customer: String;
            }

            relationships {
                has_many lines: super::line::Line [fk: order_id];
            }

            actions {
                read read { primary; }

                create place {
                    accept [customer];
                    argument lines: Vec<FieldMap> [default: Vec::new()];
                    change manage_relationship(lines, create);
                }

                update amend {
                    require_atomic false;
                    argument lines: Vec<FieldMap>;
                    change manage_relationship(lines, direct_control);
                }

                generic shout {
                    argument word: String;
                    argument times: i64 [default: 1];
                    returns String;
                    run |input| async move {
                        if input.word.is_empty() {
                            return Err(Error::Invalid("nothing to shout".into()));
                        }
                        Ok(input.word.to_uppercase().repeat(input.times as usize))
                    };
                }

                generic tally {
                    returns i64;
                    run |input| async move { Ok(Order::query(input.ctx).count().await? as i64) };
                }

                generic ping {
                    run |_input| async move { Ok(()) };
                }
            }
        }
    }
}

use line::Line;
use order::Order;

fn schema() -> async_graphql::dynamic::Schema {
    AshGraphQL::from_resources(&[&Order::DEF, &Line::DEF])
        .query_action::<Order, Memory>("tally_orders", "tally")
        .query_action::<Order, Memory>("shout", "shout")
        .mutation_action::<Order, Memory>("shout_loudly", "shout")
        .mutation_action::<Order, Memory>("ping", "ping")
        .finish::<Memory>()
        .unwrap()
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
async fn generic_actions_run_as_queries_and_mutations() {
    let ctx = Context::new(Memory::new());
    assert_eq!(data(&ctx, r#"{ shout(word: "hey") }"#).await, json!({ "shout": "HEY" }));
    assert_eq!(data(&ctx, r#"{ shout(word: "hey", times: 2) }"#).await, json!({ "shout": "HEYHEY" }));
    assert_eq!(
        data(&ctx, r#"mutation { shoutLoudly(input: { word: "hi" }) }"#).await,
        json!({ "shoutLoudly": "HI" })
    );
    // Nothing returned is `true`.
    assert_eq!(data(&ctx, "mutation { ping }").await, json!({ "ping": true }));
    assert_eq!(data(&ctx, "{ tallyOrders }").await, json!({ "tallyOrders": 0 }));

    // A failure is raised at the field, with Ash's code.
    let failed = run(&ctx, r#"{ shout(word: "") }"#).await;
    let [error] = &failed.errors[..] else { panic!("{:?}", failed.errors) };
    assert_eq!(serde_json::to_value(&error.path).unwrap(), json!(["shout"]));
    let code = error.extensions.as_ref().and_then(|e| e.get("code")).cloned();
    assert!(code.is_some(), "{error:?}");

    let sdl = schema().sdl();
    assert!(sdl.contains("shout(word: String!, times: Int): String!"), "{sdl}");
    assert!(sdl.contains("shoutLoudly(input: ShoutLoudlyInput!): String!"), "{sdl}");
    assert!(sdl.contains("ping: Boolean!"), "{sdl}");
}

#[tokio::test]
async fn managed_relationships_take_input_objects() {
    let ctx = Context::new(Memory::new());
    let sdl = schema().sdl();
    // What the destination's create takes, but the key the relationship sets.
    assert!(sdl.contains("input OrderPlaceLinesInput {\n\titem: String!\n\tquantity: Int\n}"), "{sdl}");
    assert!(sdl.contains("lines: [OrderPlaceLinesInput!]"), "{sdl}");
    // Direct control may create, update or match by key: nothing required.
    assert!(sdl.contains("input OrderAmendLinesInput {\n\tid: ID\n\titem: String\n\tquantity: Int\n}"), "{sdl}");

    let placed = data(
        &ctx,
        r#"mutation { placeOrder(input: { customer: "Ada", lines: [{ item: "tea", quantity: 2 }, { item: "cake" }] }) { result { id customer lines(sort: [{ field: ITEM }]) { item quantity } } errors { code } } }"#,
    )
    .await;
    let result = &placed["placeOrder"]["result"];
    assert_eq!(result["lines"], json!([{ "item": "cake", "quantity": 1 }, { "item": "tea", "quantity": 2 }]));

    // Direct control: the line given by id is updated, one without is created, the rest go.
    let id = result["id"].as_str().unwrap().to_string();
    let tea = Line::query(&ctx).filter(ash_core::Filter::eq("item", "tea")).one().await.unwrap();
    let amended = data(
        &ctx,
        &format!(
            r#"mutation {{ amendOrder(id: "{id}", input: {{ lines: [{{ id: "{}", quantity: 5 }}, {{ item: "jam" }}] }}) {{ result {{ lines(sort: [{{ field: ITEM }}]) {{ item quantity }} }} errors {{ code }} }} }}"#,
            tea.id
        ),
    )
    .await;
    assert_eq!(
        amended["amendOrder"]["result"]["lines"],
        json!([{ "item": "jam", "quantity": 1 }, { "item": "tea", "quantity": 5 }]),
        "{amended}"
    );
}
