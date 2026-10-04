//! GraphQL reads, gets, and subscriptions stay inside the request's tenant, as reads
//! through the engine do.

use ash_core::{Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::{PubSub, PubSubNotifier};
use async_graphql::Request;
use futures_util::StreamExt;
use std::sync::Arc;
use uuid::Uuid;

resource! {
    Shipment {
        table "shipments";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            label: String;
        }

        actions {
            create create { primary; accept [label]; }
            read read { primary; pagination keyset: true, countable: true, required: false; }
            update relabel { primary; accept [label]; }
        }
    }
}

#[tokio::test]
async fn graphql_stays_inside_the_tenant() {
    let pubsub = PubSub::new();
    let base = Context::new(Memory::new()).with_notifier(Arc::new(PubSubNotifier::new(Arc::new(pubsub.clone()))));
    let acme = base.clone().with_tenant("acme");
    let globex = base.clone().with_tenant("globex");
    let ours = Shipment::create(&acme).label("ours").await.unwrap();
    let theirs = Shipment::create(&globex).label("theirs").await.unwrap();

    let schema = AshGraphQL::from_resources(&[&Shipment::DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    let run = |ctx: &Context<Memory>, query: String| schema.execute(Request::new(query).data(ctx.clone()));

    let listed = run(&acme, "{ listShipments { results { label } } }".into()).await;
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    assert_eq!(listed.data.into_json().unwrap()["listShipments"]["results"], serde_json::json!([{ "label": "ours" }]));

    let got = run(&acme, format!(r#"{{ getShipment(id: "{}") {{ label }} }}"#, theirs.id)).await;
    assert!(got.errors.is_empty(), "{:?}", got.errors);
    assert_eq!(got.data.into_json().unwrap()["getShipment"], serde_json::Value::Null);

    // Without a tenant, a tenant-scoped read is refused, as it is outside GraphQL.
    let refused = run(&base, "{ listShipments { results { label } } }".into()).await;
    assert!(!refused.errors.is_empty());

    // Subscribers only hear about their own tenant. A subscription starts listening when
    // it is first polled, so poll each before anything changes.
    let listen = |ctx: &Context<Memory>| {
        let mut stream = schema.execute_stream(Request::new("subscription { shipmentUpdated { updated { label } } }").data(ctx.clone()));
        tokio::spawn(async move {
            let first = stream.next().await.unwrap();
            first.data.into_json().unwrap()["shipmentUpdated"]["updated"]["label"].clone()
        })
    };
    let acme_hears = listen(&acme);
    let globex_hears = listen(&globex);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    Shipment::get(&globex, theirs.id).await.unwrap().relabel_on(&globex).label("moved").await.unwrap();
    Shipment::get(&acme, ours.id).await.unwrap().relabel_on(&acme).label("kept").await.unwrap();

    let wait = |task| tokio::time::timeout(std::time::Duration::from_secs(5), task);
    assert_eq!(wait(acme_hears).await.unwrap().unwrap(), "kept");
    assert_eq!(wait(globex_hears).await.unwrap().unwrap(), "moved");
}
