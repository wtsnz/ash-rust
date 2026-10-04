//! AshTypescript's Phoenix channel transport: a Phoenix socket (`/socket/websocket`)
//! where a client joins an `ash_typescript_rpc:*` topic and pushes `run` and `validate`,
//! as AshTypescript's generated channel functions do, each answered as `POST /rpc/run`
//! and `POST /rpc/validate` answer, in the reply Phoenix sends a push. It speaks Phoenix's
//! socket serializers: `vsn=2.0.0`'s arrays (phoenix.js's default) and `1.0.0`'s objects.

use std::collections::HashMap;
use std::sync::Arc;

use ash_core::{Context, TransactionSupport};
use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::routing::get;
use serde_json::{Value as Json, json};

use super::Rpc;

/// The topics a client may join, as AshTypescript's channel is mounted.
const TOPIC_PREFIX: &str = "ash_typescript_rpc:";

/// The context a socket's pushes run in, made of its upgrade request's headers and its
/// params (Phoenix's `connect` params, such as an auth token, come as the query string).
type MakeContext<D> = Arc<dyn Fn(&HeaderMap, &HashMap<String, String>) -> Context<D> + Send + Sync>;

struct Served<D> {
    rpc: Rpc<D>,
    context: MakeContext<D>,
}

/// The Phoenix socket for `rpc` at `/socket/websocket`, each connection running in the
/// context `context` makes of its headers and params, as an `AshTypescriptRpcChannel`
/// runs actions as its socket's actor and tenant.
pub fn rpc_channel_router<D: TransactionSupport + 'static>(
    rpc: Rpc<D>,
    context: impl Fn(&HeaderMap, &HashMap<String, String>) -> Context<D> + Send + Sync + 'static,
) -> Router {
    let served = Arc::new(Served { rpc, context: Arc::new(context) });
    Router::new().route("/socket/websocket", get(upgrade::<D>)).with_state(served)
}

async fn upgrade<D: TransactionSupport + 'static>(
    State(served): State<Arc<Served<D>>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    upgrade: WebSocketUpgrade,
) -> Response {
    let ctx = (served.context)(&headers, &params);
    upgrade.on_upgrade(move |socket| serve(socket, served, ctx))
}

/// A message as Phoenix's serializers frame it, and which one framed it.
struct Frame {
    join_ref: Json,
    reference: Json,
    topic: String,
    event: String,
    payload: Json,
    arrays: bool,
}

impl Frame {
    fn parse(text: &str) -> Option<Self> {
        match serde_json::from_str::<Json>(text).ok()? {
            // `[join_ref, ref, topic, event, payload]`
            Json::Array(parts) if parts.len() == 5 => Some(Self {
                join_ref: parts[0].clone(),
                reference: parts[1].clone(),
                topic: parts[2].as_str()?.to_string(),
                event: parts[3].as_str()?.to_string(),
                payload: parts[4].clone(),
                arrays: true,
            }),
            Json::Object(map) => Some(Self {
                join_ref: map.get("join_ref").cloned().unwrap_or(Json::Null),
                reference: map.get("ref").cloned().unwrap_or(Json::Null),
                topic: map.get("topic")?.as_str()?.to_string(),
                event: map.get("event")?.as_str()?.to_string(),
                payload: map.get("payload").cloned().unwrap_or(Json::Null),
                arrays: false,
            }),
            _ => None,
        }
    }

    /// The reply to this message, framed as it was.
    fn reply(&self, status: &str, response: Json) -> String {
        self.message("phx_reply", json!({ "status": status, "response": response }))
    }

    fn message(&self, event: &str, payload: Json) -> String {
        let message = if self.arrays {
            json!([self.join_ref, self.reference, self.topic, event, payload])
        } else {
            json!({ "join_ref": self.join_ref, "ref": self.reference, "topic": self.topic, "event": event, "payload": payload })
        };
        message.to_string()
    }
}

async fn serve<D: TransactionSupport + 'static>(mut socket: WebSocket, served: Arc<Served<D>>, ctx: Context<D>) {
    let mut joined: Vec<String> = Vec::new();
    while let Some(Ok(message)) = socket.recv().await {
        let text = match message {
            Message::Text(text) => text.to_string(),
            Message::Close(_) => break,
            _ => continue,
        };
        let Some(frame) = Frame::parse(&text) else {
            continue;
        };
        let mut replies = Vec::new();
        match (frame.topic.as_str(), frame.event.as_str()) {
            ("phoenix", "heartbeat") => replies.push(frame.reply("ok", json!({}))),
            (topic, "phx_join") if topic.starts_with(TOPIC_PREFIX) => {
                if !joined.contains(&frame.topic) {
                    joined.push(frame.topic.clone());
                }
                replies.push(frame.reply("ok", json!({})));
            }
            (_, "phx_join") => replies.push(frame.reply("error", json!({ "reason": "unmatched topic" }))),
            (topic, _) if !joined.iter().any(|joined| joined == topic) => {
                replies.push(frame.reply("error", json!({ "reason": "unmatched topic" })))
            }
            (_, "phx_leave") => {
                joined.retain(|topic| *topic != frame.topic);
                replies.push(frame.reply("ok", json!({})));
                replies.push(frame.message("phx_close", json!({})));
            }
            (_, "run") => replies.push(frame.reply("ok", served.rpc.run(&ctx, &frame.payload).await)),
            (_, "validate") => replies.push(frame.reply("ok", served.rpc.validate(&ctx, &frame.payload).await)),
            (_, event) => replies.push(frame.reply(
                "error",
                json!({ "reason": format!("Unknown event: {event}"), "payload": frame.payload }),
            )),
        }
        for reply in replies {
            if socket.send(Message::Text(reply.into())).await.is_err() {
                return;
            }
        }
    }
}
