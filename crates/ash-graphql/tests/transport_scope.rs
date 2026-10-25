#![cfg(feature = "axum")]

use std::sync::Arc;
use std::time::Duration;

use ash_core::{Actor, Resource, Runtime};
use ash_graphql::AshGraphQL;
use ash_graphql::axum::{BoundRequest, RequestData, SchemaContext, SubscriptionHub};
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use axum::http::HeaderMap;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
use tower::ServiceExt;
use uuid::Uuid;

ash_core::resource! {
    Ticket {
        table "transport_scope_tickets";
        multitenancy { strategy: attribute; attribute: org; }
        attributes {
            id: Uuid [pk];
            org: String;
            owner: Uuid;
            title: String;
        }
        policies { policy always { authorize_if relates_to_actor(owner); } }
        actions {
            read read { primary; pagination keyset: true, countable: true, required: false; }
            create create { primary; accept [owner, title]; }
        }
    }
}

struct ByHeaders(Runtime<Memory>);

impl RequestData for ByHeaders {
    fn connection_init(
        &self,
        headers: &HeaderMap,
        params: Option<&Value>,
    ) -> ash_core::Result<HeaderMap> {
        let Some(params) = params.filter(|value| !value.is_null()) else {
            return Ok(headers.clone());
        };
        let params = params.as_object().ok_or(ash_core::Error::Forbidden)?;
        let mut bound = headers.clone();
        for (name, value) in params {
            if !["x-actor", "x-org"].contains(&name.as_str()) {
                return Err(ash_core::Error::Forbidden);
            }
            let value = value.as_str().ok_or(ash_core::Error::Forbidden)?;
            if (name == "x-actor" && Uuid::parse_str(value).is_err()) || value.is_empty() {
                return Err(ash_core::Error::Forbidden);
            }
            if headers.get(name).is_some_and(|header| header != value) {
                return Err(ash_core::Error::Forbidden);
            }
            bound.insert(
                axum::http::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| ash_core::Error::Forbidden)?,
                value.parse().map_err(|_| ash_core::Error::Forbidden)?,
            );
        }
        Ok(bound)
    }

    fn apply(&self, headers: &HeaderMap, request: async_graphql::Request) -> BoundRequest {
        let header = |name: &str| headers.get(name).and_then(|value| value.to_str().ok());
        let actor = header("x-actor")
            .and_then(|value| Uuid::parse_str(value).ok())
            .map(Actor::new);
        let tenant = header("x-org").map(str::to_owned);
        BoundRequest::new(request, self.0.bind(actor, tenant))
    }
}

const QUERY: &str = "{ listTickets { results { title } } }";

async fn app(bound: bool) -> (axum::Router, Uuid, Runtime<Memory>) {
    let pubsub = PubSub::new();
    let runtime = Runtime::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let alice = Uuid::new_v4();
    for (org, owner, title) in [
        ("a", alice, "ours"),
        ("a", Uuid::new_v4(), "another actor"),
        ("b", alice, "another tenant"),
    ] {
        Ticket::create(&runtime.system().bind(None, Some(org.into())))
            .owner(owner)
            .title(title)
            .await
            .unwrap();
    }
    // If a transport drops binding, this deliberately privileged default would leak.
    let schema = AshGraphQL::from_resources(&[&Ticket::DEF])
        .with_pubsub(pubsub)
        .finish_with_context(runtime.system().bind(None, Some("a".into())))
        .unwrap();
    let binder: Arc<dyn RequestData> = if bound {
        Arc::new(ByHeaders(runtime.clone()))
    } else {
        Arc::new(SchemaContext)
    };
    (
        ash_graphql::axum::graphql_router_with(
            schema,
            Arc::new(SubscriptionHub::default()),
            binder,
        ),
        alice,
        runtime,
    )
}

#[tokio::test]
async fn http_uses_bound_actor_and_tenant_and_rejects_missing_binding() {
    for bound in [true, false] {
        let (app, actor, _) = app(bound).await;
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/graphql")
            .header("content-type", "application/json")
            .header("x-org", "a")
            .header("x-actor", actor.to_string())
            .body(axum::body::Body::from(
                json!({ "query": QUERY }).to_string(),
            ))
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_binding(&serde_json::from_slice::<Value>(&body).unwrap(), bound);
    }
}

fn assert_binding(response: &Value, bound: bool) {
    if bound {
        assert_eq!(
            response["data"]["listTickets"]["results"],
            json!([{ "title": "ours" }]),
            "{response}"
        );
    } else {
        assert!(response["data"].is_null(), "{response}");
        assert!(
            response["errors"][0]["message"]
                .as_str()
                .unwrap()
                .contains("not bound"),
            "{response}"
        );
    }
}

async fn websocket_binding(protocol: &str, bound: bool, init_credentials: bool) {
    let (app, actor, _) = app(bound).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut request = format!("ws://{addr}/graphql/ws")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("sec-websocket-protocol", protocol.parse().unwrap());
    if !init_credentials {
        request.headers_mut().insert("x-org", "a".parse().unwrap());
        request
            .headers_mut()
            .insert("x-actor", actor.to_string().parse().unwrap());
    }
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            json!({ "type": "connection_init", "payload": if init_credentials { json!({"x-actor": actor.to_string(), "x-org":"a"}) } else { Value::Null } }).to_string().into(),
        ))
        .await
        .unwrap();
    let ack = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(ack.to_text().unwrap()).unwrap()["type"],
        "connection_ack"
    );
    let legacy = protocol == "graphql-ws";
    socket.send(Message::Text(json!({ "id": "1", "type": if legacy { "start" } else { "subscribe" }, "payload": { "query": QUERY } }).to_string().into())).await.unwrap();
    let response = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let response: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
    assert_eq!(response["type"], if legacy { "data" } else { "next" });
    assert_binding(&response["payload"], bound);
    socket.close(None).await.unwrap();
    server.abort();
}

#[tokio::test]
async fn both_ws_protocols_use_the_http_binder() {
    for protocol in ["graphql-transport-ws", "graphql-ws"] {
        websocket_binding(protocol, true, false).await;
    }
}

#[tokio::test]
async fn both_ws_protocols_reject_missing_binding_instead_of_schema_default() {
    for protocol in ["graphql-transport-ws", "graphql-ws"] {
        websocket_binding(protocol, false, false).await;
    }
}

#[tokio::test]
async fn both_ws_protocols_bind_connection_init_credentials() {
    for protocol in ["graphql-transport-ws", "graphql-ws"] {
        websocket_binding(protocol, true, true).await;
    }
}

#[test]
fn connection_init_rejects_invalid_conflicting_and_unsupported_credentials() {
    let binder = ByHeaders(Runtime::new(Memory::new()));
    let actor = Uuid::new_v4();
    let mut headers = HeaderMap::new();
    headers.insert("x-actor", actor.to_string().parse().unwrap());
    headers.insert("x-org", "a".parse().unwrap());
    for params in [
        json!({"x-actor":"invalid"}),
        json!({"x-org":"b"}),
        json!({"unknown":"secret"}),
        json!([]),
    ] {
        assert!(binder.connection_init(&headers, Some(&params)).is_err());
    }
    assert!(
        SchemaContext
            .connection_init(&headers, Some(&json!({"token":"unsupported"})))
            .is_err()
    );
}

#[tokio::test]
async fn both_ws_protocols_subscribe_with_connection_init_actor_and_tenant() {
    for protocol in ["graphql-transport-ws", "graphql-ws"] {
        let (app, actor, runtime) = app(true).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut request = format!("ws://{addr}/graphql/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket.send(Message::Text(json!({"type":"connection_init","payload":{"x-actor":actor.to_string(),"x-org":"a"}}).to_string().into())).await.unwrap();
        assert_eq!(socket_json(&mut socket).await["type"], "connection_ack");
        socket.send(Message::Text(json!({"id":"events","type":if protocol == "graphql-ws" {"start"} else {"subscribe"},"payload":{"query":"subscription { ticketCreated { created { title } } }"}}).to_string().into())).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        for (tenant, owner, title) in [
            ("b", actor, "foreign tenant"),
            ("a", Uuid::new_v4(), "foreign owner"),
            ("a", actor, "bound event"),
        ] {
            Ticket::create(&runtime.system().bind(None, Some(tenant.into())))
                .owner(owner)
                .title(title)
                .await
                .unwrap();
        }
        let response = socket_json(&mut socket).await;
        assert_eq!(
            response["type"],
            if protocol == "graphql-ws" {
                "data"
            } else {
                "next"
            }
        );
        assert_eq!(
            response["payload"]["data"]["ticketCreated"]["created"]["title"], "bound event",
            "{response}"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), socket.next())
                .await
                .is_err()
        );
        socket.close(None).await.unwrap();
        server.abort();
    }
}

async fn socket_json(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Value {
    let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn both_ws_protocols_reject_conflicting_init_credentials_before_acknowledgment() {
    for protocol in ["graphql-transport-ws", "graphql-ws"] {
        let (app, _, _) = app(true).await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut request = format!("ws://{addr}/graphql/ws")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("sec-websocket-protocol", protocol.parse().unwrap());
        request.headers_mut().insert("x-org", "a".parse().unwrap());
        let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"connection_init","payload":{"x-org":"b"}})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(frame, Message::Close(Some(close)) if u16::from(close.code) == 4403));
        server.abort();
    }
}
