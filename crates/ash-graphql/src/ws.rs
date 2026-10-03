//! Subscriptions over WebSocket, resolved once for every subscriber that shares them.
//!
//! Absinthe, which serves AshGraphql's subscriptions, deduplicates them: subscribers to the
//! same document in the same context share one execution, so each event is resolved once
//! and the result pushed to all of them. This does the same for `graphql-transport-ws`.
//! Subscribers to the same document with the same operation name and variables, running as
//! the same actor in the same tenant, share one stream: each event is resolved and
//! serialized once, and every subscriber receives the same text. Who a connection runs as
//! comes from its upgrade request's headers, through the router's
//! [`RequestData`](crate::axum::RequestData), as Absinthe gives each socket its own
//! context.
//!
//! Queries and mutations sent over the socket run for their sender alone. Every frame
//! matches async-graphql's own handler: `next` for each response, errors included, then
//! `complete`. The older `graphql-ws` protocol is still served by that handler.
//!
//! Buffers are bounded. A subscriber that falls more than [`SHARED_BUFFER`] events behind
//! its stream, because its connection is slow, is told how many it missed
//! (`MISSED_EVENTS`) and then completed, as a subscriber that falls behind the pubsub is,
//! so its client can resubscribe and re-read.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use async_graphql::dynamic::Schema;
use async_graphql::parser::types::{DocumentOperations, OperationType};
use async_graphql::{Request, Variables};
use async_graphql_axum::{GraphQLProtocol, GraphQLWebSocket};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// Events a shared stream holds for subscribers that haven't taken them yet.
pub const SHARED_BUFFER: usize = 4096;

/// Frames a connection queues for its socket before its subscriptions wait on it.
const CONNECTION_BUFFER: usize = 1024;

/// Upgrades a request to `/graphql/ws`: `graphql-transport-ws` with shared subscriptions
/// when the client offers it, otherwise async-graphql's handler for `graphql-ws`.
pub(crate) fn upgrade(
    protocol: GraphQLProtocol,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
    schema: Schema,
    hub: Arc<Hub>,
    request_data: Arc<dyn crate::axum::RequestData>,
) -> Response {
    let transport_ws = headers
        .get_all("sec-websocket-protocol")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|offered| offered.trim().eq_ignore_ascii_case("graphql-transport-ws"));
    if transport_ws {
        upgrade
            .protocols(["graphql-transport-ws"])
            .on_upgrade(move |socket| serve(socket, schema, hub, headers, request_data))
    } else {
        upgrade
            .protocols(["graphql-ws"])
            .on_upgrade(move |socket| GraphQLWebSocket::new(socket, schema, protocol).serve())
    }
}

#[derive(Deserialize)]
struct ClientMessage {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    payload: Option<Value>,
}

#[derive(Deserialize)]
struct SubscribePayload {
    query: String,
    #[serde(rename = "operationName")]
    operation_name: Option<String>,
    variables: Option<Value>,
}

impl SubscribePayload {
    fn request(&self) -> Request {
        let mut request = Request::new(self.query.clone());
        if let Some(name) = &self.operation_name {
            request = request.operation_name(name.clone());
        }
        if let Some(variables) = &self.variables {
            request = request.variables(Variables::from_json(variables.clone()));
        }
        request
    }

    /// The stream this subscription can share, if it's a subscription.
    fn key(&self, scope: &str) -> Option<Key> {
        let document = async_graphql::parser::parse_query(&self.query).ok()?;
        let operation = match (&document.operations, &self.operation_name) {
            (DocumentOperations::Single(operation), _) => operation,
            (DocumentOperations::Multiple(operations), Some(name)) => {
                operations.get(name.as_str())?
            }
            (DocumentOperations::Multiple(_), None) => return None,
        };
        (operation.node.ty == OperationType::Subscription).then(|| Key {
            scope: scope.to_string(),
            query: self.query.clone(),
            operation_name: self.operation_name.clone(),
            variables: self
                .variables
                .as_ref()
                .map(Value::to_string)
                .unwrap_or_default(),
        })
    }
}

/// What makes two subscriptions the same, so they can share a stream.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// Who the subscription runs as (see [`RequestData`](crate::axum::RequestData)):
    /// subscribers in different scopes never share a stream.
    scope: String,
    query: String,
    operation_name: Option<String>,
    variables: String,
}

/// What a shared stream sends its subscribers.
#[derive(Clone, Debug)]
enum Frame {
    /// A response, serialized once for all of them.
    Next(Arc<str>),
    /// The stream ended.
    Complete,
}

struct Shared {
    sender: broadcast::Sender<Frame>,
    generation: u64,
    task: JoinHandle<()>,
}

/// The shared subscription streams of one router.
#[derive(Default)]
pub struct Hub {
    streams: Mutex<HashMap<Key, Shared>>,
    generations: std::sync::atomic::AtomicU64,
}

impl Hub {
    /// How many shared streams are running.
    pub fn streams(&self) -> usize {
        self.streams.lock().expect("hub lock").len()
    }

    /// Joins the stream for `key`, starting it if nobody shares it yet.
    fn join(self: &Arc<Self>, schema: &Schema, key: Key, request: Request) -> Member {
        let mut streams = self.streams.lock().expect("hub lock");
        let receiver = match streams.get(&key) {
            Some(shared) => shared.sender.subscribe(),
            None => {
                let (sender, receiver) = broadcast::channel(SHARED_BUFFER);
                let generation = self
                    .generations
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let task = tokio::spawn(run_shared(
                    schema.clone(),
                    request,
                    sender.clone(),
                    Arc::downgrade(self),
                    key.clone(),
                    generation,
                ));
                streams.insert(
                    key.clone(),
                    Shared {
                        sender,
                        generation,
                        task,
                    },
                );
                receiver
            }
        };
        Member {
            receiver: Some(receiver),
            hub: Arc::clone(self),
            key,
        }
    }

    /// Stops the stream for `key` once nobody is subscribed to it.
    fn leave(&self, key: &Key) {
        let mut streams = self.streams.lock().expect("hub lock");
        if let Some(shared) = streams.get(key)
            && shared.sender.receiver_count() == 0
        {
            shared.task.abort();
            streams.remove(key);
        }
    }

    /// Forgets the stream for `key` that's ending, unless it's been replaced already.
    fn ended(&self, key: &Key, generation: u64) {
        let mut streams = self.streams.lock().expect("hub lock");
        if streams.get(key).is_some_and(|shared| shared.generation == generation) {
            streams.remove(key);
        }
    }
}

/// Runs one execution of a subscription and sends each response, serialized once, to
/// everyone sharing it.
async fn run_shared(
    schema: Schema,
    request: Request,
    sender: broadcast::Sender<Frame>,
    hub: Weak<Hub>,
    key: Key,
    generation: u64,
) {
    let mut stream = schema.execute_stream(request);
    while let Some(response) = stream.next().await {
        let json = serde_json::to_string(&response).expect("a response serializes");
        if sender.send(Frame::Next(json.into())).is_err() {
            break;
        }
    }
    if let Some(hub) = hub.upgrade() {
        hub.ended(&key, generation);
    }
    let _ = sender.send(Frame::Complete);
}

/// One subscriber's place in a shared stream. Leaving, however the subscription ends,
/// stops the stream if it was the last.
struct Member {
    receiver: Option<broadcast::Receiver<Frame>>,
    hub: Arc<Hub>,
    key: Key,
}

impl Member {
    async fn recv(&mut self) -> Result<Frame, broadcast::error::RecvError> {
        self.receiver.as_mut().expect("a receiver until dropped").recv().await
    }
}

impl Drop for Member {
    fn drop(&mut self) {
        drop(self.receiver.take());
        self.hub.leave(&self.key);
    }
}

fn frame(id: &str, kind: &str, payload: Option<&str>) -> Message {
    let id = serde_json::to_string(id).expect("an id serializes");
    let text = match payload {
        Some(payload) => format!(r#"{{"id":{id},"type":"{kind}","payload":{payload}}}"#),
        None => format!(r#"{{"id":{id},"type":"{kind}"}}"#),
    };
    Message::Text(text.into())
}

/// The response a subscriber gets when it falls behind its shared stream: the same
/// `MISSED_EVENTS` error a subscriber that falls behind the pubsub gets.
fn missed(count: u64) -> String {
    serde_json::json!({
        "data": null,
        "errors": [{
            "message": format!("Missed {count} events: the subscriber fell behind"),
            "extensions": { "code": "MISSED_EVENTS", "missed": count },
        }],
    })
    .to_string()
}

/// Runs one subscription for a connection until it completes or is cancelled.
async fn subscribe(
    id: String,
    payload: SubscribePayload,
    request: Request,
    scope: String,
    schema: Schema,
    hub: Arc<Hub>,
    out: mpsc::Sender<Message>,
) {
    match payload.key(&scope) {
        Some(key) => {
            let mut member = hub.join(&schema, key, request);
            loop {
                match member.recv().await {
                    Ok(Frame::Next(json)) => {
                        if out.send(frame(&id, "next", Some(&json))).await.is_err() {
                            return;
                        }
                    }
                    Ok(Frame::Complete) | Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(count)) => {
                        let _ = out.send(frame(&id, "next", Some(&missed(count)))).await;
                        break;
                    }
                }
            }
        }
        // A query or mutation runs once, for this subscriber alone.
        None => {
            let mut stream = schema.execute_stream(request);
            while let Some(response) = stream.next().await {
                let json = serde_json::to_string(&response).expect("a response serializes");
                if out.send(frame(&id, "next", Some(&json))).await.is_err() {
                    return;
                }
            }
        }
    }
    let _ = out.send(frame(&id, "complete", None)).await;
}

/// Speaks `graphql-transport-ws` on one connection.
async fn serve(
    socket: WebSocket,
    schema: Schema,
    hub: Arc<Hub>,
    headers: HeaderMap,
    request_data: Arc<dyn crate::axum::RequestData>,
) {
    let (mut sink, mut incoming) = socket.split();
    let (out, mut queued) = mpsc::channel::<Message>(CONNECTION_BUFFER);
    let writer = tokio::spawn(async move {
        while let Some(message) = queued.recv().await {
            let closing = matches!(message, Message::Close(_));
            if sink.send(message).await.is_err() || closing {
                break;
            }
        }
    });
    let close = |code: u16, reason: &str| {
        Message::Close(Some(axum::extract::ws::CloseFrame {
            code,
            reason: reason.to_string().into(),
        }))
    };

    let mut subscriptions: HashMap<String, JoinHandle<()>> = HashMap::new();
    let mut acknowledged = false;
    while let Some(Ok(message)) = incoming.next().await {
        let text = match message {
            Message::Text(text) => text,
            Message::Close(_) => break,
            _ => continue,
        };
        let Ok(message) = serde_json::from_str::<ClientMessage>(&text) else {
            let _ = out.send(close(4400, "Invalid message")).await;
            break;
        };
        match message.kind.as_str() {
            "connection_init" => {
                if acknowledged {
                    let _ = out.send(close(4429, "Too many initialisation requests")).await;
                    break;
                }
                acknowledged = true;
                let ack = Message::Text(r#"{"type":"connection_ack"}"#.into());
                if out.send(ack).await.is_err() {
                    break;
                }
            }
            "ping" => {
                let pong = match &message.payload {
                    Some(payload) => format!(r#"{{"type":"pong","payload":{payload}}}"#),
                    None => r#"{"type":"pong"}"#.to_string(),
                };
                let _ = out.send(Message::Text(pong.into())).await;
            }
            "subscribe" => {
                if !acknowledged {
                    let _ = out.send(close(4401, "Unauthorized")).await;
                    break;
                }
                let (Some(id), Some(payload)) = (message.id, message.payload) else {
                    let _ = out.send(close(4400, "Invalid message")).await;
                    break;
                };
                let Ok(payload) = serde_json::from_value::<SubscribePayload>(payload) else {
                    let _ = out.send(close(4400, "Invalid message")).await;
                    break;
                };
                subscriptions.retain(|_, task| !task.is_finished());
                if let Some(previous) = subscriptions.remove(&id) {
                    previous.abort();
                }
                let (request, scope) = request_data.apply(&headers, payload.request());
                let task = tokio::spawn(subscribe(
                    id.clone(),
                    payload,
                    request,
                    scope,
                    schema.clone(),
                    Arc::clone(&hub),
                    out.clone(),
                ));
                subscriptions.insert(id, task);
            }
            "complete" => {
                if let Some(task) = message.id.and_then(|id| subscriptions.remove(&id)) {
                    task.abort();
                }
            }
            _ => {}
        }
    }

    for task in subscriptions.into_values() {
        task.abort();
    }
    drop(out);
    let _ = writer.await;
}
