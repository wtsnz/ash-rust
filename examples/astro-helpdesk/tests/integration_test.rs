use std::process::Command;
use astro_helpdesk::{build_app, emit_typescript_sdk};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
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
    assert!(body.get("errors").is_none(), "GraphQL query failed: {body:?}");

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
        body["data"]["changeStatusTicket"]["result"]["status"].as_str().unwrap(),
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
    let temp_dir = tempfile::tempdir().unwrap();
    let sdk_path = temp_dir.path().join("ash.ts");
    emit_typescript_sdk(&sdk_path).expect("Failed to emit TypeScript SDK");
    assert!(sdk_path.exists());

    let content = std::fs::read_to_string(&sdk_path).unwrap();
    assert!(content.contains("export interface Ticket {"));
    assert!(content.contains("export interface Representative {"));
    assert!(content.contains("export const OpenTicketInputSchema = z.object({"));
    assert!(content.contains("export class TicketClient {"));

    // Check with tsc if available
    if let Ok(output) = Command::new("tsc")
        .arg("--noEmit")
        .arg("--target")
        .arg("ES2022")
        .arg("--moduleResolution")
        .arg("node")
        .arg("--skipLibCheck")
        .arg(&sdk_path)
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
