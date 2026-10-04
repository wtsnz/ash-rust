//! The HTTP and GraphQL API: signing in, reading as a line, writing through GraphQL,
//! and live updates, plus the generated TypeScript SDK.

mod support;

use std::sync::Arc;

use ash_authentication::JwtService;
use ash_core::Date;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use http_body_util::BodyExt;
use lagrange::ops;
use serde_json::{Value, json};
use support::{FleetDb, Harness};
use tower::ServiceExt;

async fn call(router: &axum::Router, path: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/json");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

async fn token(router: &axum::Router, email: &str) -> String {
    let (status, body) = call(router, "/auth/sign-in", None, json!({ "email": email, "password": lagrange::seed::PASSWORD })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    body["token"].as_str().unwrap().to_string()
}

async fn graphql(router: &axum::Router, token: Option<&str>, query: &str) -> Value {
    let (status, body) = call(router, "/graphql", token, json!({ "query": query })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["errors"].is_null(), "{body}");
    body["data"].clone()
}

fn names(list: &Value, field: &str) -> Vec<String> {
    let mut names: Vec<String> = list.as_array().unwrap().iter().map(|item| item[field].as_str().unwrap().to_string()).collect();
    names.sort();
    names
}

async fn http_api<D: FleetDb>(h: Harness<D>) {
    let app = Arc::new(h.app);
    let router = lagrange::server::router(Arc::clone(&app), Arc::new(JwtService::new("lagrange-test-signing-key-0123456789"))).unwrap();

    let (status, _) = call(&router, "/auth/sign-in", None, json!({ "email": "ada@helios-freight.example", "password": "nope" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let ada = token(&router, "ada@helios-freight.example").await;
    let mae = token(&router, "mae@red-dust-logistics.example").await;
    let grace = token(&router, "grace@helios-freight.example").await;

    // The map is public; ships belong to a line.
    let ports = graphql(&router, None, "{ listPorts { code name } }").await;
    assert_eq!(ports["listPorts"].as_array().unwrap().len(), 7);
    let ships = graphql(&router, Some(&ada), "{ listShips { name } }").await;
    assert_eq!(names(&ships["listShips"], "name"), ["Behemoth", "Long Haul", "Tin Kettle"]);
    let ships = graphql(&router, Some(&mae), "{ listShips { name } }").await;
    assert_eq!(names(&ships["listShips"], "name"), ["Dust Devil"]);
    // Ships belong to a line, and an anonymous request has none.
    let (_, refused) = call(&router, "/graphql", None, json!({ "query": "{ listShips { name } }" })).await;
    assert!(refused["errors"].to_string().contains("tenant"), "{refused}");
    let (status, _) = call(&router, "/graphql", Some("not-a-token"), json!({ "query": "{ listPorts { code } }" })).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // Relationships load as the request reads, batched per request: the map is public
    // but its berths are for crew, and a ship's captain is crew of its line.
    let leo_berths = |token: Option<String>| {
        let router = router.clone();
        async move {
            let map = graphql(&router, token.as_deref(), "{ listPorts { code planet { name } berths { code } } }").await;
            let leo = map["listPorts"].as_array().unwrap().iter().find(|port| port["code"] == "LEO").unwrap().clone();
            assert_eq!(leo["planet"]["name"], "Earth");
            leo["berths"].as_array().unwrap().len()
        }
    };
    assert_eq!(leo_berths(None).await, 0);
    assert_eq!(leo_berths(Some(ada.clone())).await, 4);
    let crewed = graphql(&router, Some(&ada), "{ listShips { name captain { name } } }").await;
    let mut captains: Vec<(String, Value)> = crewed["listShips"]
        .as_array()
        .unwrap()
        .iter()
        .map(|ship| (ship["name"].as_str().unwrap().to_string(), ship["captain"]["name"].clone()))
        .collect();
    captains.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        captains,
        [
            ("Behemoth".to_string(), Value::Null),
            ("Long Haul".to_string(), json!("Yuri")),
            ("Tin Kettle".to_string(), json!("Valentina")),
        ]
    );

    // A shipper books through GraphQL; the booking joins their line.
    let booked = graphql(
        &router,
        Some(&grace),
        &format!(
            r#"mutation {{ bookContract(input: {{
                externalRef: "GQL-1", shipperEmail: "grace@helios-freight.example",
                originPortId: "{}", destinationPortId: "{}",
                freightCredits: 7000, insuredValue: "125.50", deliverBy: "2187-09-01"
            }}) {{ errors {{ message }} result {{ id line status insuredValue }} }} }}"#,
            h.sol.port("KSC").id,
            h.sol.port("OLY").id
        ),
    )
    .await;
    let result = &booked["bookContract"];
    assert_eq!(result["errors"], json!([]), "{booked}");
    assert_eq!(result["result"]["line"], lagrange::seed::HELIOS);
    assert_eq!(result["result"]["status"], "booked");
    // Red Dust doesn't see it.
    let theirs = graphql(&router, Some(&mae), "{ listContracts { externalRef } }").await;
    assert!(theirs["listContracts"].as_array().unwrap().is_empty());
}
on_every_backend!(http_api);

/// Subscribers hear about their own line's voyages, and only those.
async fn live_voyage_updates<D: FleetDb>(h: Harness<D>) {
    let schema = lagrange::server::schema(&h.app).unwrap();
    // A subscription starts listening when first polled, so poll before anything moves.
    let listen = |name: &str| {
        let ctx = h
            .app
            .registry_context()
            .with_actor(h.ctx(name).actor.clone().unwrap())
            .with_tenant(h.ctx(name).tenant().unwrap().to_string());
        let mut stream = schema.execute_stream(async_graphql::Request::new("subscription { voyageUpdated { updated { id status } } }").data(ctx));
        tokio::spawn(async move { stream.next().await.map(|response| response.data.into_json().unwrap()) })
    };
    let helios = listen("Ada");
    let red_dust = listen("Mae");
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    let voyage = ops::plan_voyage(h.ctx("Ada"), h.sol.ship("Long Haul"), h.sol.port("LEO").id, h.sol.port("PZZ").id, Date::parse("2187-02-01").unwrap())
        .await
        .unwrap();
    ops::clear_customs(&h.customs(lagrange::seed::HELIOS), voyage.id).await.unwrap();

    let heard = tokio::time::timeout(std::time::Duration::from_secs(5), helios)
        .await
        .expect("the line's dispatcher hears about it")
        .unwrap()
        .unwrap();
    assert_eq!(heard["voyageUpdated"]["updated"]["status"], "cleared", "{heard}");
    assert_eq!(heard["voyageUpdated"]["updated"]["id"], voyage.id.to_string());

    let leaked = tokio::time::timeout(std::time::Duration::from_millis(300), red_dust).await;
    assert!(leaked.is_err(), "another line must not hear about it: {leaked:?}");
}
on_every_backend!(live_voyage_updates);
