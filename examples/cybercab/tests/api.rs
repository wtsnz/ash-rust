//! The command center's API: reads with their relationships, the fleet moving live over
//! the WebSocket, and operator commands held to the state machines.

use std::sync::Arc;
use std::time::Duration;

use ash_memory::Memory;
use ash_pubsub::PubSub;
use axum::Router;
use axum::body::Body;
use axum::http::Request;
use cybercab::city::City;
use cybercab::server::{context, router};
use cybercab::sim::{SimConfig, Simulation};
use futures_util::{SinkExt, StreamExt};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tower::ServiceExt;

struct Room {
    app: Router,
    sim: Simulation<Memory>,
}

async fn room() -> Room {
    let pubsub = PubSub::new();
    let ctx = context(Memory::new(), &pubsub);
    let city = City::austin();
    cybercab::seed::austin(&ctx, &city, 3).await.unwrap();
    let config = SimConfig {
        speedup: 30.0,
        demand: 2.0,
        seed: 3,
    };
    let sim = Simulation::new(ctx.clone(), Arc::clone(&city), config)
        .await
        .unwrap();
    Room {
        app: router(ctx, pubsub).unwrap(),
        sim,
    }
}

impl Room {
    async fn graphql(&self, query: &str) -> Value {
        let request = Request::post("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(json!({ "query": query }).to_string()))
            .unwrap();
        let response = self.app.clone().oneshot(request).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let response: Value = serde_json::from_slice(&body).unwrap();
        assert!(response["errors"].is_null(), "{response}");
        response["data"].clone()
    }

    /// Serves the room on a real socket, for the WebSocket.
    async fn listen(&self) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = self.app.clone();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        addr.to_string()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_carry_their_relationships() {
    let room = room().await;
    let data = room
        .graphql(
            r#"{
            listTrips(filter: { status: { eq: "completed" } }, sort: [{ field: requested_at, order: DESC }], limit: 5) {
                code fare_cents rider { display_name tier } cab { call_sign } zone { name }
            }
            listCabs { call_sign status battery_pct trips_completed }
        }"#,
    )
    .await;
    let trips = data["listTrips"].as_array().unwrap();
    assert_eq!(trips.len(), 5);
    assert!(
        trips
            .iter()
            .all(|t| t["rider"]["display_name"].is_string() && t["cab"]["call_sign"].is_string())
    );
    assert_eq!(
        data["listCabs"].as_array().unwrap().len(),
        cybercab::seed::FLEET_SIZE
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_fleet_moves_live_over_the_websocket() {
    let mut room = room().await;
    let addr = room.listen().await;
    let mut request = format!("ws://{addr}/graphql/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "graphql-transport-ws".parse().unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let send = |value: Value| Message::Text(value.to_string().into());
    socket
        .send(send(json!({ "type": "connection_init" })))
        .await
        .unwrap();
    let ack = socket.next().await.unwrap().unwrap();
    assert!(ack.to_text().unwrap().contains("connection_ack"), "{ack}");
    socket
        .send(send(json!({
            "id": "cabs",
            "type": "subscribe",
            "payload": { "query": "subscription { cabUpdated { call_sign lng lat speed_kph } }" },
        })))
        .await
        .unwrap();

    // The protocol doesn't acknowledge a subscription, so keep the fleet moving until the
    // updates arrive.
    let mut moving = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while moving < 3 && tokio::time::Instant::now() < deadline {
        room.sim.step().await.unwrap();
        while let Ok(Some(message)) =
            tokio::time::timeout(Duration::from_millis(200), socket.next()).await
        {
            let message: Value = serde_json::from_str(message.unwrap().to_text().unwrap()).unwrap();
            if message["type"] == "next"
                && message["payload"]["data"]["cabUpdated"]["speed_kph"].as_i64() > Some(0)
            {
                moving += 1;
            }
        }
    }
    assert!(moving >= 3, "heard {moving} moving cabs");
}

#[tokio::test(flavor = "multi_thread")]
async fn operators_command_cabs_through_the_api() {
    let room = room().await;
    let cabs = room
        .graphql(r#"{ listCabs(filter: { status: { eq: "available" } }, limit: 1) { id } }"#)
        .await;
    let id = cabs["listCabs"][0]["id"].as_str().unwrap();
    let recalled = room
        .graphql(
        &format!(r#"mutation {{ recallCab(input: {{ id: "{id}" }}) {{ success errors {{ message }} result {{ status }} }} }}"#),
    )
    .await;
    assert_eq!(
        recalled["recallCab"]["result"]["status"], "returning",
        "{recalled}"
    );
    // The state machine refuses what the cab's state doesn't allow.
    let refused = room
        .graphql(
        &format!(r#"mutation {{ beginRideCab(input: {{ id: "{id}" }}) {{ success errors {{ message }} }} }}"#),
    )
    .await;
    assert_eq!(refused["beginRideCab"]["success"], false, "{refused}");
}
