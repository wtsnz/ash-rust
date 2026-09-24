use std::time::Duration;
use ash_core::{ActionDef, AttrType, AttributeDef, Context, ResourceDef};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::PubSub;
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
    let ctx = Context::new(memory);

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

    let mut mut_req = Request::new(mutation).data(ctx.clone());
    mut_req = mut_req.data(pubsub.clone());
    let res = schema.execute(mut_req).await;
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

#[tokio::test]
async fn test_phase6_subscription_filtered() {
    let pubsub = PubSub::new();
    let memory = Memory::new();
    let ctx = Context::new(memory);

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
    let mut req1 = Request::new(open_mutation).data(ctx.clone());
    req1 = req1.data(pubsub.clone());
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
    let mut req2 = Request::new(closed_mutation).data(ctx.clone());
    req2 = req2.data(pubsub.clone());
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
    let ctx = Context::new(memory);

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
    let res = schema.execute(Request::new(create_mutation).data(ctx.clone()).data(pubsub.clone())).await;
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
    let res_up = schema.execute(Request::new(&update_mut).data(ctx.clone()).data(pubsub.clone())).await;
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
    let res_del = schema.execute(Request::new(&destroy_mut).data(ctx.clone()).data(pubsub.clone())).await;
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
