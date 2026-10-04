//! A request runs as its headers say, through the router's `RequestData`: a query over
//! HTTP in its tenant, and each subscription on a socket in its connection's tenant, never
//! sharing a stream with another tenant's subscribers.
#![cfg(feature = "axum")]

use std::sync::Arc;
use std::time::Duration;

use ash_core::{Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_graphql::axum::{RequestData, SubscriptionHub};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use axum::http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
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

/// The tenant is the `x-org` header.
struct ByOrg(Context<Memory>);

impl RequestData for ByOrg {
    fn apply(&self, headers: &HeaderMap, request: async_graphql::Request) -> (async_graphql::Request, String) {
        let org = headers.get("x-org").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        (request.data(self.0.clone().with_tenant(org.clone())), org)
    }
}

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn subscribe(addr: std::net::SocketAddr, org: &str) -> Socket {
    let mut request = format!("ws://{addr}/graphql/ws").into_client_request().unwrap();
    request.headers_mut().insert("Sec-WebSocket-Protocol", "graphql-transport-ws".parse().unwrap());
    request.headers_mut().insert("x-org", org.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    let send = |value: serde_json::Value| Message::Text(value.to_string().into());
    socket.send(send(serde_json::json!({ "type": "connection_init" }))).await.unwrap();
    assert_eq!(read(&mut socket).await["type"], "connection_ack");
    let query = "subscription { shipmentUpdated { updated { label } } }";
    socket
        .send(send(serde_json::json!({ "id": "1", "type": "subscribe", "payload": { "query": query } })))
        .await
        .unwrap();
    socket
}

async fn read(socket: &mut Socket) -> serde_json::Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("a message within 5 s")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}

async fn streams(hub: &SubscriptionHub, expected: usize) {
    let started = std::time::Instant::now();
    while hub.streams() != expected {
        assert!(started.elapsed() < Duration::from_secs(5), "{} streams, not {expected}", hub.streams());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn requests_run_in_the_tenant_their_headers_name() {
    let pubsub = PubSub::new();
    let base = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let (acme, globex) = (base.clone().with_tenant("acme"), base.clone().with_tenant("globex"));
    let ours = Shipment::create(&acme).label("ours").await.unwrap();
    let theirs = Shipment::create(&globex).label("theirs").await.unwrap();

    let schema = AshGraphQL::from_resources(&[&Shipment::DEF])
        .with_pubsub(pubsub)
        .finish_with_context(base.clone())
        .unwrap();
    let hub = Arc::new(SubscriptionHub::default());
    let router = ash_graphql::axum::graphql_router_with(schema, hub.clone(), Arc::new(ByOrg(base.clone())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // Over HTTP, a list reads the tenant the header names.
    let listed: serde_json::Value = reqwest_post(addr, "acme", "{ listShipments { results { label } } }").await;
    assert_eq!(listed["data"]["listShipments"]["results"], serde_json::json!([{ "label": "ours" }]), "{listed}");

    // On sockets, the same subscription in two tenants runs twice, each hearing its own.
    let mut acme_socket = subscribe(addr, "acme").await;
    let mut globex_socket = subscribe(addr, "globex").await;
    streams(&hub, 2).await;
    theirs.relabel_on(&globex).label("moved").await.unwrap();
    ours.relabel_on(&acme).label("kept").await.unwrap();
    assert_eq!(read(&mut globex_socket).await["payload"]["data"]["shipmentUpdated"]["updated"]["label"], "moved");
    assert_eq!(read(&mut acme_socket).await["payload"]["data"]["shipmentUpdated"]["updated"]["label"], "kept");
}

/// Posts `query` to `/graphql` as `org`.
async fn reqwest_post(addr: std::net::SocketAddr, org: &str, query: &str) -> serde_json::Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = serde_json::json!({ "query": query }).to_string();
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let request = format!(
        "POST /graphql HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nx-org: {org}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let json = response.split("\r\n\r\n").nth(1).unwrap();
    serde_json::from_str(json.trim()).unwrap_or_else(|_| panic!("a JSON body: {response}"))
}
