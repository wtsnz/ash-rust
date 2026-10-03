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
                    results {
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

    let tickets = body["data"]["listTickets"]["results"].as_array().unwrap();
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
                        fields
                        message
                    }
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
    assert_eq!(payload["errors"], serde_json::json!([]));
    let created_id = payload["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(payload["result"]["title"].as_str().unwrap(), new_title);

    // 4. Update (Change status via GraphQL mutation)
    let update_mutation = serde_json::json!({
        "query": r#"
            mutation ChangeStatus($id: ID!, $input: ChangeStatusTicketInput) {
                changeStatusTicket(id: $id, input: $input) {
                    result {
                        id
                        status
                    }
                    errors {
                        message
                    }
                }
            }
        "#,
        "variables": {
            "id": created_id,
            "input": {
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
            mutation Close($id: ID!) {
                closeTicket(id: $id) {
                    result {
                        id
                    }
                    errors {
                        message
                    }
                }
            }
        "#,
        "variables": {
            "id": created_id
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
    assert_eq!(body["data"]["closeTicket"]["errors"], serde_json::json!([]), "{body:?}");
    assert_eq!(body["data"]["closeTicket"]["result"]["id"], created_id.as_str());
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

/// The input fields of a GraphQL input type, by name, with their types.
async fn input_fields(app: &axum::Router, name: &str) -> Vec<(String, serde_json::Value)> {
    let named = r#"kind name enumValues { name }"#;
    let query = serde_json::json!({
        "query": format!(
            "{{ __type(name: \"{name}\") {{ inputFields {{ name type {{ {named} \
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
    let mut fields: Vec<(String, serde_json::Value)> = body["data"]["__type"]["inputFields"]
        .as_array()
        .unwrap_or_else(|| panic!("{name} missing from GraphQL schema: {body:?}"))
        .iter()
        .map(|field| (field["name"].as_str().unwrap().to_string(), field["type"].clone()))
        .collect();
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    fields
}

/// A TypeScript interface's `name?: type;` fields, as (name, type), sorted.
fn ts_fields(ts: &str, header: &str) -> Vec<(String, String)> {
    let body = ts
        .split(&format!("{header} {{\n"))
        .nth(1)
        .unwrap_or_else(|| panic!("`{header}` missing from generated TypeScript"));
    let mut fields: Vec<(String, String)> = body
        .lines()
        .take_while(|line| *line != "}")
        .filter(|line| !line.trim_start().starts_with("/**"))
        .map(|line| {
            let (name, ty) = line.trim().split_once("?: ").unwrap();
            (name.to_string(), ty.trim_end_matches(';').to_string())
        })
        .collect();
    fields.sort();
    fields
}

/// GraphQL's type with non-null stripped, and whether it's a list.
fn unwrap_type(mut gql: &serde_json::Value) -> (&serde_json::Value, bool) {
    if gql["kind"] == "NON_NULL" {
        gql = &gql["ofType"];
    }
    let list = gql["kind"] == "LIST";
    if list {
        gql = &gql["ofType"];
        if gql["kind"] == "NON_NULL" {
            gql = &gql["ofType"];
        }
    }
    (gql, list)
}

/// A TypeScript value type matches the GraphQL type it stands for.
fn assert_value_type(ts_type: &str, gql: &serde_json::Value, at: &str) {
    let gql_named = gql["name"].as_str().unwrap();
    let base = ts_type
        .trim_end_matches(" | null")
        .trim_end_matches("[]")
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_end_matches(" | null");
    match base {
        "string" => assert!(
            gql["kind"] == "SCALAR" && !["Int", "Float", "Boolean"].contains(&gql_named),
            "{at}"
        ),
        "number" => assert!(["Int", "Float"].contains(&gql_named), "{at}"),
        "boolean" => assert_eq!(gql_named, "Boolean", "{at}"),
        literals if literals.starts_with('"') => {
            assert_eq!(gql["kind"], "ENUM", "{at}");
            let mut ts_values: Vec<&str> = literals.split(" | ").map(|v| v.trim_matches('"')).collect();
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
        other => panic!("unexpected TypeScript type `{other}` at {at}"),
    }
}

#[tokio::test]
async fn test_typescript_filters_match_graphql_filter_inputs() {
    let app = build_app().await.expect("Failed to build Axum app");
    let mut ts = ash_typescript::types::generate_common_types();
    for def in [&astro_helpdesk::TICKET_DEF, &astro_helpdesk::REPRESENTATIVE_DEF] {
        ts.push_str(&ash_typescript::types::generate_resource_filter_input(def));
    }
    let ash_filter = ts_fields(&ts, "export interface AshFilter<T>");
    let text_ops = ts_fields(&ts, "export interface AshTextFilter extends AshFilter<string>");

    for resource in ["Ticket", "Representative"] {
        let input = format!("{resource}FilterInput");
        let ts_input = ts_fields(&ts, &format!("export interface {input}"));
        let gql_input = input_fields(&app, &input).await;
        let ts_names: Vec<&str> = ts_input.iter().map(|(name, _)| name.as_str()).collect();
        let gql_names: Vec<&str> = gql_input.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(ts_names, gql_names, "{input} does not match");

        for ((field, ts_type), (_, gql_type)) in ts_input.iter().zip(&gql_input) {
            let at = format!("{input}.{field}: {ts_type} vs {gql_type}");
            let (gql, gql_list) = unwrap_type(gql_type);
            let gql_named = gql["name"].as_str().unwrap();
            // `and` / `or` / `not`, and relationships: another filter input.
            if let Some(other) = ts_type.strip_suffix("FilterInput[]").or(ts_type.strip_suffix("FilterInput")) {
                assert_eq!(gql_list, ts_type.ends_with("[]"), "list mismatch at {at}");
                assert_eq!(gql_named, format!("{other}FilterInput"), "{at}");
                continue;
            }
            // A field: AshGraphql's operators, over its value type.
            assert!(!gql_list, "{at}");
            let (value_type, text) = match ts_type.as_str() {
                "AshTextFilter" => ("string", true),
                other => (
                    other
                        .strip_prefix("AshFilter<")
                        .and_then(|t| t.strip_suffix('>'))
                        .unwrap_or_else(|| panic!("unexpected filter type at {at}")),
                    false,
                ),
            };
            let ops = input_fields(&app, gql_named).await;
            let mut ts_ops: Vec<&str> = ash_filter.iter().map(|(op, _)| op.as_str()).collect();
            if text {
                ts_ops.extend(text_ops.iter().map(|(op, _)| op.as_str()));
            }
            ts_ops.sort_unstable();
            let gql_ops: Vec<&str> = ops.iter().map(|(op, _)| op.as_str()).collect();
            assert_eq!(ts_ops, gql_ops, "{at}: operators");
            let (eq, _) = unwrap_type(&ops.iter().find(|(op, _)| op == "eq").unwrap().1);
            assert_value_type(value_type, eq, &format!("{at}: eq"));
        }
    }
}
