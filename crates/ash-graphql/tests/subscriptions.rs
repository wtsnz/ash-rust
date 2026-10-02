use std::time::Duration;
use ash_core::{ActionDef, AttrType, AttributeDef, Context, ResourceDef};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use std::sync::Arc;
use async_graphql::Request;
use futures_util::StreamExt;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::optional("status", AttrType::Atom { one_of: &["OPEN", "CLOSED"] }),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["title", "status"]),
    ActionDef::update("update").accept(&["title", "status"]),
    ActionDef::destroy("destroy"),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: &[],
    actions: TICKET_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[tokio::test]
async fn test_phase6_subscription_stream_with_pubsub() {
    let pubsub = PubSub::new();
    let memory = Memory::new();
    // Writes publish through the context's notifier.
    let ctx = Context::new(memory).with_pubsub(Arc::new(pubsub.clone()));

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub.clone())
        .finish::<Memory>()
        .expect("Failed to build schema with subscriptions");

    // 1. Verify schema introspection includes Subscription root
    let introspection = schema
        .execute(
            r#"
        query {
            __schema {
                subscriptionType {
                    name
                    fields {
                        name
                    }
                }
            }
        }
    "#,
        )
        .await;

    let json = introspection.data.into_json().unwrap();
    let sub_fields = json["__schema"]["subscriptionType"]["fields"]
        .as_array()
        .unwrap();
    let names: Vec<&str> = sub_fields
        .iter()
        .filter_map(|f| f["name"].as_str())
        .collect();
    assert!(names.contains(&"ticketCreated"));
    assert!(names.contains(&"ticketUpdated"));
    assert!(names.contains(&"ticketDestroyed"));

    // 2. Start a subscription stream for ticketCreated
    let sub_query = r#"
        subscription {
            ticketCreated {
                title
                status
            }
        }
    "#;

    let req = Request::new(sub_query).data(ctx.clone());
    let mut stream = schema.execute_stream(req);

    let sub_handle = tokio::spawn(async move { stream.next().await });

    // Give subscription a moment to poll and attach subscriber
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Execute create mutation that triggers pubsub publish
    let mutation = r#"
        mutation {
            createTicket(input: { title: "Urgent Outage", status: OPEN }) {
                success
                result {
                    id
                    title
                }
            }
        }
    "#;

    let res = schema.execute(Request::new(mutation).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Mutation errors: {:?}", res.errors);

    // Receive from subscription stream
    let event = tokio::time::timeout(Duration::from_secs(2), sub_handle)
        .await
        .expect("Subscription timed out")
        .expect("Join error")
        .expect("Stream ended unexpectedly");

    assert!(
        event.errors.is_empty(),
        "Stream event errors: {:?}",
        event.errors
    );
    let event_json = event.data.into_json().unwrap();
    assert_eq!(event_json["ticketCreated"]["title"], "Urgent Outage");
    assert_eq!(event_json["ticketCreated"]["status"], "OPEN");
}

/// Every write is published once, through the context's notifier, whether it came
/// through GraphQL or not.
#[tokio::test]
async fn each_write_is_heard_once() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub.clone())
        .finish::<Memory>()
        .unwrap();

    let mut stream =
        schema.execute_stream(Request::new("subscription { ticketCreated { title } }").data(ctx.clone()));
    let heard = tokio::spawn(async move {
        let mut titles = Vec::new();
        while let Ok(Some(event)) = tokio::time::timeout(Duration::from_millis(300), stream.next()).await {
            titles.push(event.data.into_json().unwrap()["ticketCreated"]["title"].clone());
        }
        titles
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    // A `PubSub` in the request data, which mutations used to publish to themselves,
    // doesn't publish the change again.
    let mutation = r#"mutation { createTicket(input: { title: "From GraphQL" }) { success } }"#;
    let res = schema.execute(Request::new(mutation).data(ctx.clone()).data(pubsub.clone())).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let mut input = ash_core::FieldMap::new();
    input.insert("title".into(), ash_core::Value::String("From Rust".into()));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();

    assert_eq!(heard.await.unwrap(), ["From GraphQL", "From Rust"]);
}

#[tokio::test]
async fn test_phase6_subscription_filtered() {
    let pubsub = PubSub::new();
    let memory = Memory::new();
    // Writes publish through the context's notifier.
    let ctx = Context::new(memory).with_pubsub(Arc::new(pubsub.clone()));

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub.clone())
        .finish::<Memory>()
        .expect("Failed to build schema with subscriptions");

    // Subscription looking ONLY for CLOSED tickets
    let sub_query = r#"
        subscription {
            ticketCreated(filter: { status: { eq: CLOSED } }) {
                title
                status
            }
        }
    "#;

    let req = Request::new(sub_query).data(ctx.clone());
    let mut stream = schema.execute_stream(req);

    let sub_handle = tokio::spawn(async move { stream.next().await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // 1. Create an OPEN ticket (should NOT trigger subscription)
    let open_mutation = r#"
        mutation {
            createTicket(input: { title: "Open Ticket", status: OPEN }) {
                success
            }
        }
    "#;
    let req1 = Request::new(open_mutation).data(ctx.clone());
    let res1 = schema.execute(req1).await;
    assert!(res1.errors.is_empty());

    // 2. Create a CLOSED ticket (SHOULD trigger subscription)
    let closed_mutation = r#"
        mutation {
            createTicket(input: { title: "Resolved Ticket", status: CLOSED }) {
                success
            }
        }
    "#;
    let req2 = Request::new(closed_mutation).data(ctx.clone());
    let res2 = schema.execute(req2).await;
    assert!(res2.errors.is_empty());

    // Await subscription event
    let event = tokio::time::timeout(Duration::from_secs(2), sub_handle)
        .await
        .expect("Subscription timed out")
        .expect("Join error")
        .expect("Stream ended unexpectedly");

    let event_json = event.data.into_json().unwrap();
    assert_eq!(event_json["ticketCreated"]["title"], "Resolved Ticket");
    assert_eq!(event_json["ticketCreated"]["status"], "CLOSED");
}

#[tokio::test]
async fn test_phase6_subscription_updated_and_destroyed() {
    let pubsub = PubSub::new();
    let memory = Memory::new();
    // Writes publish through the context's notifier.
    let ctx = Context::new(memory).with_pubsub(Arc::new(pubsub.clone()));

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub.clone())
        .finish::<Memory>()
        .expect("Failed to build schema with subscriptions");

    // Create a ticket first
    let create_mutation = r#"
        mutation {
            createTicket(input: { title: "Original Title", status: OPEN }) {
                success
                result {
                    id
                }
            }
        }
    "#;
    let res = schema.execute(Request::new(create_mutation).data(ctx.clone())).await;
    let ticket_id = res.data.into_json().unwrap()["createTicket"]["result"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Subscribe to ticketUpdated
    let update_sub = format!(
        r#"
        subscription {{
            ticketUpdated(id: "{ticket_id}") {{
                title
                status
            }}
        }}
    "#
    );
    let mut stream_up = schema.execute_stream(Request::new(&update_sub).data(ctx.clone()));
    let update_handle = tokio::spawn(async move { stream_up.next().await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Execute update mutation
    let update_mut = format!(
        r#"
        mutation {{
            updateTicket(input: {{ id: "{ticket_id}", title: "Updated Title", status: CLOSED }}) {{
                success
            }}
        }}
    "#
    );
    let res_up = schema.execute(Request::new(&update_mut).data(ctx.clone())).await;
    assert!(res_up.errors.is_empty());

    let event_up = tokio::time::timeout(Duration::from_secs(2), update_handle)
        .await
        .expect("Update subscription timed out")
        .expect("Join error")
        .expect("Stream ended unexpectedly");
    let event_up_json = event_up.data.into_json().unwrap();
    assert_eq!(event_up_json["ticketUpdated"]["title"], "Updated Title");
    assert_eq!(event_up_json["ticketUpdated"]["status"], "CLOSED");

    // Subscribe to ticketDestroyed
    let destroy_sub = format!(
        r#"
        subscription {{
            ticketDestroyed(id: "{ticket_id}")
        }}
    "#
    );
    let mut stream_del = schema.execute_stream(Request::new(&destroy_sub).data(ctx.clone()));
    let destroy_handle = tokio::spawn(async move { stream_del.next().await });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Execute destroy mutation
    let destroy_mut = format!(
        r#"
        mutation {{
            destroyTicket(input: {{ id: "{ticket_id}" }}) {{
                success
            }}
        }}
    "#
    );
    let res_del = schema.execute(Request::new(&destroy_mut).data(ctx.clone())).await;
    assert!(res_del.errors.is_empty());

    let event_del = tokio::time::timeout(Duration::from_secs(2), destroy_handle)
        .await
        .expect("Destroy subscription timed out")
        .expect("Join error")
        .expect("Stream ended unexpectedly");
    let event_del_json = event_del.data.into_json().unwrap();
    assert_eq!(event_del_json["ticketDestroyed"], ticket_id);
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn test_phase6_axum_router_endpoints() {
    use axum::body::Body;
    use axum::http::{Request as HttpRequest, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let _memory = Memory::new();
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    let app = ash_graphql::axum::graphql_router(schema);

    // 1. GET /graphiql returns HTML
    let req_graphiql = HttpRequest::builder()
        .uri("/graphiql")
        .body(Body::empty())
        .unwrap();

    let res_graphiql = app.clone().oneshot(req_graphiql).await.unwrap();
    assert_eq!(res_graphiql.status(), StatusCode::OK);
    let body_bytes = res_graphiql.into_body().collect().await.unwrap().to_bytes();
    let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
    assert!(body_str.contains("GraphiQL IDE"));

    // 2. POST /graphql executes query
    let query_payload = serde_json::json!({
        "query": "{ schema_version }"
    });

    let req_post = HttpRequest::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&query_payload).unwrap()))
        .unwrap();

    let res_post = app.oneshot(req_post).await.unwrap();
    assert_eq!(res_post.status(), StatusCode::OK);
    let post_body = res_post.into_body().collect().await.unwrap().to_bytes();
    let res_json: serde_json::Value = serde_json::from_slice(&post_body).unwrap();
    assert_eq!(res_json["data"]["schema_version"], "ash-graphql-0.1.0");
}

/// The router serves subscriptions at `/graphql/ws`, the endpoint its GraphiQL points at,
/// over the `graphql-transport-ws` protocol that generated TypeScript clients speak.
#[cfg(feature = "axum")]
#[tokio::test]
async fn router_serves_subscriptions_over_websocket() {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish_with_context(ctx.clone())
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, ash_graphql::axum::graphql_router(schema)).await.unwrap();
    });

    let mut request = format!("ws://{addr}/graphql/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", "graphql-transport-ws".parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    let send = |value: serde_json::Value| Message::Text(value.to_string().into());
    socket.send(send(serde_json::json!({ "type": "connection_init" }))).await.unwrap();
    let ack = read_message(&mut socket).await;
    assert_eq!(ack["type"], "connection_ack");
    socket
        .send(send(serde_json::json!({
            "id": "1",
            "type": "subscribe",
            "payload": { "query": "subscription { ticketCreated { title } }" },
        })))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut input = ash_core::FieldMap::new();
    input.insert("title".into(), ash_core::Value::String("Over the wire".into()));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();

    let next = read_message(&mut socket).await;
    assert_eq!(next["type"], "next", "{next}");
    assert_eq!(next["id"], "1");
    assert_eq!(next["payload"]["data"]["ticketCreated"]["title"], "Over the wire");
}

#[cfg(feature = "axum")]
async fn read_message<S>(socket: &mut S) -> serde_json::Value
where
    S: futures_util::Stream<
            Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
        > + Unpin,
{
    use futures_util::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    loop {
        match tokio::time::timeout(Duration::from_secs(5), socket.next()).await.unwrap() {
            Some(Ok(Message::Text(text))) => return serde_json::from_str(&text).unwrap(),
            Some(Ok(_)) => continue,
            other => panic!("socket closed: {other:?}"),
        }
    }
}

/// An optional argument given as `null`, as an unset variable is, means no argument.
#[tokio::test]
async fn null_subscription_arguments_mean_none() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    let request = Request::new(
        "subscription ($filter: TicketFilterInput) { ticketCreated(filter: $filter) { title } }",
    )
    .variables(async_graphql::Variables::from_json(serde_json::json!({ "filter": null })))
    .data(ctx.clone());
    let mut stream = schema.execute_stream(request);
    let heard = tokio::spawn(async move { stream.next().await });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut input = ash_core::FieldMap::new();
    input.insert("title".into(), ash_core::Value::String("Unfiltered".into()));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();

    let event = tokio::time::timeout(Duration::from_secs(2), heard).await.unwrap().unwrap().unwrap();
    assert!(event.errors.is_empty(), "{:?}", event.errors);
    assert_eq!(event.data.into_json().unwrap()["ticketCreated"]["title"], "Unfiltered");
}

async fn next<S: futures_util::Stream + Unpin>(stream: &mut S) -> Option<S::Item> {
    tokio::time::timeout(Duration::from_secs(2), stream.next()).await.unwrap()
}

/// A subscriber that falls further behind than the pubsub buffers is told how many events
/// it missed, rather than its subscription silently ending, so it can resubscribe and
/// re-read what it holds.
#[tokio::test]
async fn a_subscriber_that_falls_behind_is_told_what_it_missed() {
    let pubsub = PubSub::with_capacity(4);
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish::<Memory>()
        .unwrap();
    let create = |title: String| {
        let ctx = ctx.clone();
        async move {
            let mut input = ash_core::FieldMap::new();
            input.insert("title".into(), ash_core::Value::String(title));
            ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();
        }
    };
    let mut stream =
        schema.execute_stream(Request::new("subscription { ticketCreated { title } }").data(ctx.clone()));

    // Subscribed: the first create is heard.
    let first = tokio::spawn(async move {
        let event = next(&mut stream).await;
        (stream, event)
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    create("First".into()).await;
    let (mut stream, event) = first.await.unwrap();
    assert_eq!(event.unwrap().data.into_json().unwrap()["ticketCreated"]["title"], "First");

    // Twenty creates while the subscriber isn't reading overflow its buffer of four.
    for i in 0..20 {
        create(format!("Burst {i}")).await;
    }
    let lagged = next(&mut stream).await.expect("told it fell behind");
    assert_eq!(lagged.errors.len(), 1, "{:?}", lagged);
    let error = lagged.errors[0].extensions.as_ref().expect("extensions");
    assert_eq!(error.get("code"), Some(&async_graphql::Value::from("MISSED_EVENTS")));
    assert_eq!(error.get("missed"), Some(&async_graphql::Value::from(16)));

    // The subscription ends there; a new one hears what happens next.
    assert!(next(&mut stream).await.is_none());
    let mut stream =
        schema.execute_stream(Request::new("subscription { ticketCreated { title } }").data(ctx.clone()));
    let later = tokio::spawn(async move { next(&mut stream).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    create("Later".into()).await;
    let event = later.await.unwrap().unwrap();
    assert_eq!(event.data.into_json().unwrap()["ticketCreated"]["title"], "Later");
}

#[cfg(feature = "axum")]
type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Serves `schema` on a socket, with the hub its shared subscriptions run in.
#[cfg(feature = "axum")]
async fn serve_shared(
    schema: async_graphql::dynamic::Schema,
) -> (std::net::SocketAddr, Arc<ash_graphql::axum::SubscriptionHub>) {
    let hub = Arc::new(ash_graphql::axum::SubscriptionHub::default());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = ash_graphql::axum::graphql_router_with_hub(schema, hub.clone());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (addr, hub)
}

/// Connects with `protocol` and, for `graphql-transport-ws`, completes the handshake.
#[cfg(feature = "axum")]
async fn connect(addr: std::net::SocketAddr, protocol: &str) -> Socket {
    use futures_util::SinkExt;
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let mut request = format!("ws://{addr}/graphql/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Sec-WebSocket-Protocol", protocol.parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(Message::Text(serde_json::json!({ "type": "connection_init" }).to_string().into()))
        .await
        .unwrap();
    assert_eq!(read_message(&mut socket).await["type"], "connection_ack");
    socket
}

#[cfg(feature = "axum")]
async fn send_json(socket: &mut Socket, value: serde_json::Value) {
    use futures_util::SinkExt;
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

/// Waits until the hub runs `expected` shared streams.
#[cfg(feature = "axum")]
async fn streams(hub: &ash_graphql::axum::SubscriptionHub, expected: usize) {
    let started = std::time::Instant::now();
    while hub.streams() != expected {
        assert!(started.elapsed() < Duration::from_secs(5), "{} streams, not {expected}", hub.streams());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Subscribers to the same subscription share one execution, as Absinthe deduplicates
/// AshGraphql's: each event is resolved once and every subscriber gets the same frame.
/// A different subscription runs on its own, and a shared one stops with its last
/// subscriber.
#[cfg(feature = "axum")]
#[tokio::test(flavor = "multi_thread")]
async fn subscribers_to_the_same_subscription_share_one_execution() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish_with_context(ctx.clone())
        .unwrap();
    let (addr, hub) = serve_shared(schema).await;

    let created = "subscription { ticketCreated { title } }";
    let mut sockets = Vec::new();
    for i in 0..20 {
        let mut socket = connect(addr, "graphql-transport-ws").await;
        let id = format!("watch-{i}");
        send_json(&mut socket, serde_json::json!({ "id": id, "type": "subscribe", "payload": { "query": created } })).await;
        sockets.push(socket);
    }
    let mut other = connect(addr, "graphql-transport-ws").await;
    send_json(
        &mut other,
        serde_json::json!({ "id": "statuses", "type": "subscribe", "payload": { "query": "subscription { ticketCreated { status } }" } }),
    )
    .await;
    streams(&hub, 2).await;

    let mut input = ash_core::FieldMap::new();
    input.insert("title".into(), ash_core::Value::String("Shared".into()));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();
    for (i, socket) in sockets.iter_mut().enumerate() {
        let next = read_message(socket).await;
        assert_eq!(next["id"], format!("watch-{i}"));
        assert_eq!(next["payload"]["data"]["ticketCreated"]["title"], "Shared", "{next}");
    }
    assert_eq!(read_message(&mut other).await["payload"]["data"]["ticketCreated"]["status"], serde_json::Value::Null);

    // Completing one subscriber leaves the stream to the rest; the last one stops it.
    let mut last = sockets.pop().unwrap();
    for (i, socket) in sockets.iter_mut().enumerate() {
        send_json(socket, serde_json::json!({ "id": format!("watch-{i}"), "type": "complete" })).await;
    }
    streams(&hub, 2).await;
    drop(last.close(None).await);
    streams(&hub, 1).await;
}

/// The older `graphql-ws` protocol is still served.
#[cfg(feature = "axum")]
#[tokio::test(flavor = "multi_thread")]
async fn the_older_graphql_ws_protocol_still_works() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish_with_context(ctx.clone())
        .unwrap();
    let (addr, hub) = serve_shared(schema).await;
    let mut socket = connect(addr, "graphql-ws").await;
    send_json(&mut socket, serde_json::json!({ "id": "1", "type": "start", "payload": { "query": "subscription { ticketCreated { title } }" } })).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(hub.streams(), 0, "graphql-ws isn't shared");

    let mut input = ash_core::FieldMap::new();
    input.insert("title".into(), ash_core::Value::String("Legacy".into()));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();
    let data = read_message(&mut socket).await;
    assert_eq!(data["type"], "data", "{data}");
    assert_eq!(data["payload"]["data"]["ticketCreated"]["title"], "Legacy");
}

/// A mutation sent over the socket runs for each sender: only subscriptions are shared.
#[cfg(feature = "axum")]
#[tokio::test(flavor = "multi_thread")]
async fn mutations_over_the_socket_are_not_shared() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .with_pubsub(pubsub)
        .finish_with_context(ctx.clone())
        .unwrap();
    let (addr, _) = serve_shared(schema).await;
    let mutation = r#"mutation { createTicket(input: { title: "Twice" }) { success } }"#;
    for _ in 0..2 {
        let mut socket = connect(addr, "graphql-transport-ws").await;
        send_json(&mut socket, serde_json::json!({ "id": "m", "type": "subscribe", "payload": { "query": mutation } })).await;
        let next = read_message(&mut socket).await;
        assert_eq!(next["payload"]["data"]["createTicket"]["success"], true, "{next}");
        assert_eq!(read_message(&mut socket).await["type"], "complete");
    }
    use ash_core::DataLayer;
    let stored = ctx
        .data
        .run_query(&TICKET_DEF, &ash_core::CompiledQuery::default())
        .await
        .unwrap();
    assert_eq!(stored.len(), 2);
}
