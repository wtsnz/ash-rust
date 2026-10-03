use astro_helpdesk::{TicketActions, build_app, emit_typescript_sdk};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use std::process::Command;
use tower::ServiceExt;
use uuid::Uuid;

#[tokio::test]
async fn test_fullstack_helpdesk_server_crud() {
    let app = build_app().await.expect("Failed to build Axum app");

    // 1. Health check
    let req = Request::builder()
        .method("GET")
        .uri("/health")
        .body(Body::empty())
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. Read (Query initial tickets with author relationship)
    let query = serde_json::json!({
        "query": r#"
            query {
                listTickets {
                    id
                    title
                    status
                    priority
                    author {
                        id
                        name
                        role
                    }
                }
            }
        "#
    });

    let req = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&query).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        body.get("errors").is_none(),
        "GraphQL query failed: {body:?}"
    );

    let tickets = body["data"]["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 3, "Expected 3 initial seeded tickets");
    assert!(tickets[0]["author"]["name"].is_string());

    // 3. Create (Open new ticket via GraphQL mutation)
    let new_title = format!("Critical test alert: {}", Uuid::new_v4());
    let mutation = serde_json::json!({
        "query": r#"
            mutation Open($input: OpenTicketInput!) {
                openTicket(input: $input) {
                    result {
                        id
                        title
                        status
                        priority
                    }
                    errors {
                        field
                        message
                    }
                    success
                }
            }
        "#,
        "variables": {
            "input": {
                "title": new_title,
                "description": "Integration test created ticket",
                "priority": 1,
                "status": "OPEN"
            }
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&mutation).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body.get("errors").is_none(), "Mutation errors: {body:?}");

    let payload = &body["data"]["openTicket"];
    assert!(payload["success"].as_bool().unwrap());
    let created_id = payload["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(payload["result"]["title"].as_str().unwrap(), new_title);

    // 4. Update (Change status via GraphQL mutation)
    let update_mutation = serde_json::json!({
        "query": r#"
            mutation ChangeStatus($input: ChangeStatusTicketInput!) {
                changeStatusTicket(input: $input) {
                    result {
                        id
                        status
                    }
                    success
                }
            }
        "#,
        "variables": {
            "input": {
                "id": created_id,
                "status": "RESOLVED"
            }
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&update_mutation).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        body["data"]["changeStatusTicket"]["result"]["status"]
            .as_str()
            .unwrap(),
        "RESOLVED"
    );

    // 5. Delete (Close ticket via GraphQL mutation)
    let close_mutation = serde_json::json!({
        "query": r#"
            mutation Close($input: CloseTicketInput!) {
                closeTicket(input: $input) {
                    success
                }
            }
        "#,
        "variables": {
            "input": {
                "id": created_id
            }
        }
    });

    let req = Request::builder()
        .method("POST")
        .uri("/graphql")
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&close_mutation).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(body["data"]["closeTicket"]["success"].as_bool().unwrap());
}

#[test]
fn test_typescript_codegen_and_tsc_typecheck() {
    let frontend_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
    let sdk_path = frontend_dir.join("src/lib/ash.ts");
    emit_typescript_sdk(&sdk_path).expect("Failed to emit TypeScript SDK");
    assert!(sdk_path.exists());

    let content = std::fs::read_to_string(&sdk_path).unwrap();
    assert!(content.contains("export interface Ticket {"));
    assert!(content.contains("export interface Representative {"));
    assert!(content.contains("export const OpenTicketInputSchema = z.object({"));
    assert!(content.contains("export class TicketClient {"));

    // Check with tsc inside frontend (where zod is installed in node_modules)
    if frontend_dir.join("node_modules").exists()
        && let Ok(output) = Command::new("tsc")
            .current_dir(&frontend_dir)
            .arg("--noEmit")
            .arg("--target")
            .arg("ES2022")
            .arg("--moduleResolution")
            .arg("node")
            .arg("--skipLibCheck")
            .arg("src/lib/ash.ts")
            .output()
    {
        assert!(
            output.status.success(),
            "tsc output: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[tokio::test]
async fn test_rust_resource_and_context_dsl() {
    let mem = ash_memory::Memory::new();
    let ctx = ash_core::Context::new(mem);

    // 1. Create representative using Action builder DSL
    let rep = astro_helpdesk::Representative::create(&ctx)
        .name("Alex Mercer")
        .email("alex@support.ash")
        .role("Escalations Lead")
        .await
        .expect("Failed to create representative via Action DSL");

    assert_eq!(rep.name, "Alex Mercer");

    // 2. Open ticket using Action builder DSL
    let ticket = astro_helpdesk::Ticket::open(&ctx)
        .title("Flaky websocket disconnects in EU cluster")
        .description("Heartbeat latency spikes over 5000ms.")
        .status(astro_helpdesk::TicketStatus::Open)
        .priority(2i64)
        .author_id(rep.id)
        .await
        .expect("Failed to open ticket via Action DSL");

    assert_eq!(ticket.title, "Flaky websocket disconnects in EU cluster");
    assert_eq!(ticket.status, astro_helpdesk::TicketStatus::Open);
    assert_eq!(ticket.priority, 2);
    assert_eq!(ticket.author_id, Some(rep.id));

    // 3. Query tickets using fluent Query DSL
    let tickets = astro_helpdesk::Ticket::query(&ctx)
        .filter(astro_helpdesk::Ticket::priority.lte(2))
        .all()
        .await
        .expect("Failed to query tickets via Query DSL");

    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0].id, ticket.id);

    // 4. Update ticket status using Instance Action DSL
    let updated = ticket
        .change_status(&ctx)
        .status(astro_helpdesk::TicketStatus::InProgress)
        .await
        .expect("Failed to update status via Action DSL");

    assert_eq!(updated.status, astro_helpdesk::TicketStatus::InProgress);

    // 5. Close/Destroy ticket using Instance Action DSL
    updated
        .close(&ctx)
        .await
        .expect("Failed to close ticket via Action DSL");

    let remaining = astro_helpdesk::Ticket::query(&ctx)
        .all()
        .await
        .expect("Failed to query tickets");

    assert_eq!(remaining.len(), 0);
}

#[tokio::test]
async fn test_typescript_filters_match_graphql_filter_inputs() {
    let app = build_app().await.expect("Failed to build Axum app");
    let mut ts = ash_typescript::types::generate_common_types();
    ts.push_str(&ash_typescript::types::generate_resource_filter_input(
        &astro_helpdesk::TICKET_DEF,
    ));
    ts.push_str(&ash_typescript::types::generate_resource_filter_input(
        &astro_helpdesk::REPRESENTATIVE_DEF,
    ));

    // The helpdesk has no boolean attributes, so `BooleanFilterInput` is not in its schema.
    let pairs = [
        ("UuidFilter", "UuidFilterInput"),
        ("StringFilter", "StringFilterInput"),
        ("TextFilter", "TextFilterInput"),
        ("TicketStatusFilter", "TicketstatusEnumFilterInput"),
        ("IntFilter", "IntFilterInput"),
        ("FloatFilter", "FloatFilterInput"),
        ("TicketFilterInput", "TicketFilterInput"),
        ("RepresentativeFilterInput", "RepresentativeFilterInput"),
    ];
    let named = r#"kind name enumValues { name }"#;
    for (ts_name, gql_name) in pairs {
        let body = ts
            .split(&format!("export interface {ts_name} {{\n"))
            .nth(1)
            .unwrap_or_else(|| panic!("{ts_name} missing from generated TypeScript"));
        // `name?: type;` lines, as (name, type).
        let mut ts_fields: Vec<(String, String)> = body
            .lines()
            .take_while(|line| *line != "}")
            .map(|line| {
                let (name, ty) = line.trim().split_once("?: ").unwrap();
                (name.to_string(), ty.trim_end_matches(';').to_string())
            })
            .collect();
        ts_fields.sort();

        let query = serde_json::json!({
            "query": format!(
                "{{ __type(name: \"{gql_name}\") {{ inputFields {{ name type {{ {named} \
                 ofType {{ {named} ofType {{ {named} ofType {{ {named} }} }} }} }} }} }} }}"
            )
        });
        let req = Request::builder()
            .method("POST")
            .uri("/graphql")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&query).unwrap()))
            .unwrap();
        let res = app.clone().oneshot(req).await.unwrap();
        let body_bytes = res.into_body().collect().await.unwrap().to_bytes();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        let mut gql_fields: Vec<(String, serde_json::Value)> = body["data"]["__type"]["inputFields"]
            .as_array()
            .unwrap_or_else(|| panic!("{gql_name} missing from GraphQL schema: {body:?}"))
            .iter()
            .map(|field| (field["name"].as_str().unwrap().to_string(), field["type"].clone()))
            .collect();
        gql_fields.sort_by(|a, b| a.0.cmp(&b.0));

        let ts_names: Vec<&str> = ts_fields.iter().map(|(name, _)| name.as_str()).collect();
        let gql_names: Vec<&str> = gql_fields.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(ts_names, gql_names, "{ts_name} does not match {gql_name}");

        for ((field, ts_type), (_, gql_type)) in ts_fields.iter().zip(&gql_fields) {
            let at = format!("{ts_name}.{field}: {ts_type} vs {gql_type}");
            // GraphQL: strip non-null, note a list, then strip non-null again.
            let mut gql = gql_type;
            if gql["kind"] == "NON_NULL" {
                gql = &gql["ofType"];
            }
            let gql_list = gql["kind"] == "LIST";
            if gql_list {
                gql = &gql["ofType"];
                if gql["kind"] == "NON_NULL" {
                    gql = &gql["ofType"];
                }
            }
            let ts_list = ts_type.ends_with("[]");
            assert_eq!(ts_list, gql_list, "list mismatch at {at}");
            let base = ts_type
                .trim_end_matches("[]")
                .trim_start_matches('(')
                .trim_end_matches(')');
            let gql_named = gql["name"].as_str().unwrap();
            match base {
                "string" => assert!(
                    gql["kind"] == "SCALAR"
                        && !["Int", "Float", "Boolean"].contains(&gql_named),
                    "{at}"
                ),
                "number" => assert!(["Int", "Float"].contains(&gql_named), "{at}"),
                "boolean" => assert_eq!(gql_named, "Boolean", "{at}"),
                literals if literals.starts_with('"') => {
                    assert_eq!(gql["kind"], "ENUM", "{at}");
                    let mut ts_values: Vec<&str> =
                        literals.split(" | ").map(|v| v.trim_matches('"')).collect();
                    let mut gql_values: Vec<&str> = gql["enumValues"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v["name"].as_str().unwrap())
                        .collect();
                    ts_values.sort_unstable();
                    gql_values.sort_unstable();
                    assert_eq!(ts_values, gql_values, "{at}");
                }
                filter => {
                    let expected = pairs
                        .iter()
                        .find(|(ts, _)| *ts == filter)
                        .map(|(_, gql)| *gql)
                        .unwrap_or_else(|| panic!("no GraphQL input paired with {filter} at {at}"));
                    assert_eq!(gql_named, expected, "{at}");
                }
            }
        }
    }
}
