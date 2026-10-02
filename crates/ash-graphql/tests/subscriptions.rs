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
