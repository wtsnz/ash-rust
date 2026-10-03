//! Talking to a Cybercab server as any client would: GraphQL over HTTP, and
//! subscriptions over WebSocket with `graphql-transport-ws`.

use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// A pooled HTTP client for one server's `/graphql`.
#[derive(Clone)]
pub struct Graphql {
    client: Client<HttpConnector, Full<Bytes>>,
    base: String,
}

/// What a request came back with.
pub struct Response {
    pub data: Value,
    pub elapsed: Duration,
    /// GraphQL errors, mutation errors, or an HTTP failure.
    pub failed: bool,
}

impl Graphql {
    pub fn new(base: &str) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(256)
            .build(connector);
        Self {
            client,
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    /// Runs a query or mutation. A mutation whose result reports errors counts as failed.
    pub async fn request(&self, query: &str, variables: Value) -> Response {
        let body = json!({ "query": query, "variables": variables }).to_string();
        let request = hyper::Request::post(format!("{}/graphql", self.base))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .expect("a request");
        let started = Instant::now();
        let result = async {
            let response = self.client.request(request).await.ok()?;
            let ok = response.status().is_success();
            let bytes = response.into_body().collect().await.ok()?.to_bytes();
            let value: Value = serde_json::from_slice(&bytes).ok()?;
            Some((ok, value))
        }
        .await;
        let elapsed = started.elapsed();
        match result {
            Some((ok, value)) => {
                let errors = value.get("errors").is_some_and(|e| !e.is_null());
                let data = value.get("data").cloned().unwrap_or(Value::Null);
                let mutation_failed = data
                    .as_object()
                    .map(|fields| {
                        fields.values().any(|field| {
                            field
                                .get("errors")
                                .and_then(Value::as_array)
                                .is_some_and(|errors| !errors.is_empty())
                        })
                    })
                    .unwrap_or(false);
                Response {
                    data,
                    elapsed,
                    failed: !ok || errors || mutation_failed,
                }
            }
            None => Response {
                data: Value::Null,
                elapsed,
                failed: true,
            },
        }
    }

    /// Runs a request that setup depends on, panicking if it fails.
    pub async fn must(&self, query: &str, variables: Value) -> Value {
        let response = self.request(query, variables).await;
        assert!(!response.failed, "{query} failed: {}", response.data);
        response.data
    }

    /// A GET returning JSON, such as `/metrics`.
    pub async fn get_json(&self, path: &str) -> Option<Value> {
        let request = hyper::Request::get(format!("{}{path}", self.base))
            .body(Full::new(Bytes::new()))
            .ok()?;
        let response = self.client.request(request).await.ok()?;
        if !response.status().is_success() {
            return None;
        }
        let bytes = response.into_body().collect().await.ok()?.to_bytes();
        serde_json::from_slice(&bytes).ok()
    }

    pub async fn healthy(&self) -> bool {
        let Ok(request) = hyper::Request::get(format!("{}/health", self.base)).body(Full::new(Bytes::new())) else {
            return false;
        };
        matches!(self.client.request(request).await, Ok(r) if r.status().is_success())
    }
}

/// An event a subscriber heard, or the end of its subscription.
pub enum Heard {
    Next(Value),
    /// The server sent an error, or ended the subscription.
    Ended,
}

/// Opens a `graphql-transport-ws` connection and subscribes to each of `queries`, calling
/// `on_event` with everything heard until `until`.
pub async fn subscribe(
    base: &str,
    queries: &[&str],
    until: Instant,
    mut on_event: impl FnMut(Heard),
) -> Result<(), String> {
    let url = format!("{}/graphql/ws", base.replacen("http", "ws", 1));
    let mut request = url.into_client_request().map_err(|e| e.to_string())?;
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "graphql-transport-ws".parse().expect("a header"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|e| e.to_string())?;
    let send = |value: Value| Message::Text(value.to_string().into());
    socket
        .send(send(json!({ "type": "connection_init", "payload": {} })))
        .await
        .map_err(|e| e.to_string())?;
    loop {
        let message = socket.next().await.ok_or("closed before ack")?.map_err(|e| e.to_string())?;
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
            if value["type"] == "connection_ack" {
                break;
            }
        }
    }
    for (i, query) in queries.iter().enumerate() {
        socket
            .send(send(json!({ "id": format!("s{i}"), "type": "subscribe", "payload": { "query": query } })))
            .await
            .map_err(|e| e.to_string())?;
    }
    loop {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let Ok(message) = tokio::time::timeout(left, socket.next()).await else {
            break;
        };
        let Some(Ok(message)) = message else {
            on_event(Heard::Ended);
            break;
        };
        match message {
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                match value["type"].as_str() {
                    Some("next") if value["payload"]["errors"].is_null() => {
                        on_event(Heard::Next(value["payload"]["data"].clone()))
                    }
                    Some("next") | Some("error") | Some("complete") => on_event(Heard::Ended),
                    Some("ping") => {
                        let _ = socket.send(send(json!({ "type": "pong" }))).await;
                    }
                    _ => {}
                }
            }
            Message::Ping(payload) => {
                let _ = socket.send(Message::Pong(payload)).await;
            }
            Message::Close(_) => {
                on_event(Heard::Ended);
                break;
            }
            _ => {}
        }
    }
    let _ = socket.close(None).await;
    Ok(())
}

/// How long ago `timestamp` (RFC 3339) was, in milliseconds.
pub fn age_ms(timestamp: &str) -> Option<f64> {
    let at = chrono::DateTime::parse_from_rfc3339(timestamp).ok()?;
    let now = chrono::Utc::now();
    Some((now.signed_duration_since(at).num_microseconds()? as f64) / 1000.0)
}
