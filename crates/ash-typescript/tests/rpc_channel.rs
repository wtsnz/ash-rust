//! AshTypescript's Phoenix channel transport: join an `ash_typescript_rpc:*` topic on the
//! Phoenix socket and push `run` or `validate`; each reply holds what `POST /rpc/run` or
//! `/rpc/validate` would answer.
#![cfg(feature = "axum")]

use ash_core::{Context, resource};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value as Json, json};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

resource! {
    Note {
        table "rpc_channel_notes";

        attributes {
            id: Uuid [pk];
            body: String;
        }

        actions {
            read read { primary; }
            create create { primary; accept [body]; validate string_length(body, min: 2); }
        }
    }
}

#[tokio::test]
async fn actions_run_over_the_phoenix_socket() {
    let base = Context::new(Memory::new());
    let rpc = Rpc::new().action::<Note>("list_notes", "read").action::<Note>("create_note", "create");
    let app = ash_typescript::rpc::channel::rpc_channel_router(rpc, move |_, _| base.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/socket/websocket?vsn=2.0.0")).await.unwrap();
    let mut push = async |message: Json| -> Json {
        socket.send(Message::Text(message.to_string().into())).await.unwrap();
        loop {
            if let Some(Ok(Message::Text(text))) = socket.next().await {
                return serde_json::from_str(&text).unwrap();
            }
        }
    };
    let topic = "ash_typescript_rpc:me";

    // Pushing before joining is refused, as Phoenix refuses an unjoined topic.
    let early = push(json!(["1", "1", topic, "run", { "action": "list_notes", "fields": ["body"] }])).await;
    assert_eq!(early[4]["status"], json!("error"));

    let joined = push(json!(["1", "2", topic, "phx_join", {}])).await;
    assert_eq!(joined, json!(["1", "2", topic, "phx_reply", { "status": "ok", "response": {} }]));
    let wrong = push(json!(["2", "3", "elsewhere:x", "phx_join", {}])).await;
    assert_eq!(wrong[4]["status"], json!("error"));

    let created = push(json!(["1", "4", topic, "run", { "action": "create_note", "input": { "body": "hello" }, "fields": ["body"] }])).await;
    assert_eq!(created[4], json!({ "status": "ok", "response": { "success": true, "data": { "body": "hello" } } }));
    let listed = push(json!(["1", "5", topic, "run", { "action": "list_notes", "fields": ["body"] }])).await;
    assert_eq!(listed[4]["response"]["data"], json!([{ "body": "hello" }]));
    let invalid = push(json!(["1", "6", topic, "validate", { "action": "create_note", "input": { "body": "x" } }])).await;
    assert_eq!(invalid[4]["response"]["success"], json!(false));

    let heartbeat = push(json!([null, "7", "phoenix", "heartbeat", {}])).await;
    assert_eq!(heartbeat[4]["status"], json!("ok"));
    let unknown = push(json!(["1", "8", topic, "nope", {}])).await;
    assert_eq!(unknown[4]["response"]["reason"], json!("Unknown event: nope"));
}
