//! GraphQL subscription should honor the same read policy as list queries.

use std::time::Duration;

use ash_core::{Actor, Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::PubSub;
use async_graphql::Request;
use futures_util::StreamExt;
use uuid::Uuid;

mod ticket {
    use super::*;

    resource! {
        Ticket {
        table "repro_gql_tickets";

        attributes {
            id: Uuid [pk];
            title: String;
            owner_id: Uuid;
        }

        actions {
            create create {
                primary;
                accept [title, owner_id];
            }

            read read {
                primary;
            }
        }

        policies {
            policy action_type(create) {
                authorize_if always;
            }
            policy action_type(read) {
                authorize_if relates_to_actor(owner_id);
            }
        }
    }}
}

use ticket::Ticket;

#[tokio::test]
async fn subscription_does_not_emit_rows_the_actor_cannot_read() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&Ticket::DEF])
        .with_pubsub(pubsub.clone())
        .finish::<Memory>()
        .expect("schema");

    let viewer = Actor::new(Uuid::new_v4());
    let other = Uuid::new_v4();

    let listed = schema
        .execute(
            Request::new("{ listTickets { title } }").data(ctx.clone().with_actor(viewer.clone())),
        )
        .await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);

    let mut stream = schema.execute_stream(
        Request::new("subscription { ticketCreated { title owner_id } }")
            .data(ctx.clone())
            .data(viewer.clone()),
    );
    let pending = tokio::spawn(async move { stream.next().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mutation = format!(
        r#"mutation {{
            createTicket(input: {{ title: "not yours", owner_id: "{other}" }}) {{
                success
                errors {{ message field code }}
            }}
        }}"#
    );
    let created = schema
        .execute(Request::new(mutation).data(ctx.clone()).data(pubsub))
        .await;
    assert!(created.errors.is_empty(), "{:?}", created.errors);
    let created_json = created.data.into_json().unwrap();
    assert_eq!(
        created_json["createTicket"]["success"], true,
        "create must succeed so the subscription has a chance to leak: {created_json}"
    );

    let event = tokio::time::timeout(Duration::from_secs(2), pending).await;

    assert!(
        event.is_err(),
        "subscriber received a ticket they cannot list: {:?}",
        event.unwrap().unwrap().unwrap().data.into_json()
    );
}
