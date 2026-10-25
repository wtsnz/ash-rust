#![cfg(feature = "axum")]

use ash_core::{Actor, Context, Resource, Runtime, resource};
use ash_graphql::AshGraphQL;
use ash_graphql::axum::{BoundRequest, RequestData, SubscriptionHub};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use axum::http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use uuid::Uuid;

resource! {
    Payslip {
        table "subscription_payslips";
        attributes { id: Uuid [pk]; label: String; owner_id: Uuid; salary: Option<i64>; }
        policies {
            policy action_type(create) { authorize_if always; }
            policy action_type(read) { authorize_if always; }
        }
        field_policies { field salary { authorize_if relates_to_actor(owner_id); } }
        actions {
            create create { primary; accept [label, owner_id, salary]; }
            read read { primary; }
        }
    }
}
struct Binder {
    runtime: Runtime<Memory>,
    group: Option<Context<Memory>>,
}
impl RequestData for Binder {
    fn apply(&self, headers: &HeaderMap, request: async_graphql::Request) -> BoundRequest {
        let actor = headers
            .get("x-user")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| Uuid::parse_str(value).ok())
            .map(|id| Actor::new(id).with_role("staff"));
        let member = self.runtime.bind(actor, None);
        match &self.group {
            Some(group) => BoundRequest::for_subscription_group(request, member, group.clone()),
            None => BoundRequest::new(request, member),
        }
    }
}
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
const SUB: &str = "subscription { payslipCreated { created { label salary } } }";
async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn read(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).unwrap();
        }
    }
}
async fn connect(addr: std::net::SocketAddr, user: Option<Uuid>) -> Socket {
    let mut request = format!("ws://{addr}/graphql/ws")
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "graphql-transport-ws".parse().unwrap(),
    );
    if let Some(user) = user {
        request
            .headers_mut()
            .insert("x-user", user.to_string().parse().unwrap());
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send(&mut socket, json!({"type":"connection_init"})).await;
    assert_eq!(read(&mut socket).await["type"], "connection_ack");
    socket
}
async fn streams(hub: &SubscriptionHub, expected: usize) {
    let started = std::time::Instant::now();
    while hub.streams() != expected {
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{} streams, expected {expected}",
            hub.streams()
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn subscribe(socket: &mut Socket) {
    send(
        socket,
        json!({"id":"1","type":"subscribe","payload":{"query":SUB}}),
    )
    .await;
    // An operation completes only after the server has registered the preceding stream.
    send(
        socket,
        json!({"id":"ready","type":"subscribe","payload":{"query":"{ __typename }"}}),
    )
    .await;
    assert_eq!(read(socket).await["id"], "ready");
    assert_eq!(read(socket).await["type"], "complete");
    tokio::time::sleep(Duration::from_millis(50)).await;
}
async fn setup(
    group: Option<Actor>,
) -> (
    Runtime<Memory>,
    async_graphql::dynamic::Schema,
    std::net::SocketAddr,
    Arc<SubscriptionHub>,
    tokio::task::JoinHandle<()>,
) {
    let pubsub = PubSub::new();
    let runtime = Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&Payslip::DEF])
        .with_pubsub(pubsub)
        .finish_with_context(runtime.bootstrap())
        .unwrap();
    let binder = Binder {
        runtime: runtime.clone(),
        group: group.map(|actor| runtime.bind(Some(actor), None)),
    };
    let hub = Arc::new(SubscriptionHub::default());
    let router =
        ash_graphql::axum::graphql_router_with(schema.clone(), hub.clone(), Arc::new(binder));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (runtime, schema, addr, hub, server)
}
fn created(frame: &Value) -> &Value {
    &frame["payload"]["data"]["payslipCreated"]["created"]
}

#[tokio::test(flavor = "multi_thread")]
async fn members_with_the_same_role_keep_their_own_salary_projection() {
    let (runtime, schema, addr, hub, server) = setup(None).await;
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());
    let mut a = connect(addr, Some(alice)).await;
    subscribe(&mut a).await;
    streams(&hub, 1).await;
    let mut b = connect(addr, Some(bob)).await;
    subscribe(&mut b).await;
    streams(&hub, 2).await;
    Payslip::create(&runtime.bootstrap())
        .label("alice")
        .owner_id(alice)
        .salary(Some(250_000))
        .await
        .unwrap();
    assert_eq!(created(&read(&mut a).await)["salary"], 250_000);
    assert!(created(&read(&mut b).await)["salary"].is_null());
    let control = schema
        .execute(
            async_graphql::Request::new("{ listPayslips { salary } }")
                .data(runtime.bind(Some(Actor::new(bob).with_role("staff")), None)),
        )
        .await;
    assert!(control.data.into_json().unwrap()["listPayslips"][0]["salary"].is_null());
    a.close(None).await.unwrap();
    b.close(None).await.unwrap();
    streams(&hub, 0).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_group_resolves_as_its_generic_actor_instead_of_any_member() {
    let generic = Uuid::new_v4();
    let (runtime, _, addr, hub, server) = setup(Some(Actor::new(generic).with_role("staff"))).await;
    let alice = Uuid::new_v4();
    let mut a = connect(addr, Some(alice)).await;
    subscribe(&mut a).await;
    let mut b = connect(addr, Some(Uuid::new_v4())).await;
    subscribe(&mut b).await;
    streams(&hub, 1).await;
    for (owner, expected) in [(alice, Value::Null), (generic, json!(125_000))] {
        Payslip::create(&runtime.bootstrap())
            .label("group")
            .owner_id(owner)
            .salary(Some(125_000))
            .await
            .unwrap();
        let first = read(&mut a).await;
        let second = read(&mut b).await;
        assert_eq!(first["payload"], second["payload"]);
        assert_eq!(created(&first)["salary"], expected);
    }
    a.close(None).await.unwrap();
    b.close(None).await.unwrap();
    streams(&hub, 0).await;
    server.abort();
}

#[tokio::test(flavor = "multi_thread")]
async fn anonymous_subscribers_share_a_public_scope_by_default() {
    let (runtime, _, addr, hub, server) = setup(None).await;
    let mut a = connect(addr, None).await;
    subscribe(&mut a).await;
    let mut b = connect(addr, None).await;
    subscribe(&mut b).await;
    streams(&hub, 1).await;
    Payslip::create(&runtime.bootstrap())
        .label("public")
        .owner_id(Uuid::new_v4())
        .salary(Some(500))
        .await
        .unwrap();
    let first = read(&mut a).await;
    let second = read(&mut b).await;
    assert_eq!(first["payload"], second["payload"]);
    assert_eq!(created(&first)["label"], "public");
    assert!(created(&first)["salary"].is_null());
    a.close(None).await.unwrap();
    b.close(None).await.unwrap();
    streams(&hub, 0).await;
    server.abort();
}
