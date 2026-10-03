//! Sends both desks the same requests, as an admin, an agent and a viewer of one org, and
//! compares what they answer: the same records, in the same order, with the same fields
//! visible, and the same failures.
//!
//! ```bash
//! parity --rust http://127.0.0.1:4701 --elixir http://127.0.0.1:4702 --fixture fixture.json
//! ```
//!
//! Both must have loaded the same fixture. Writes change both desks the same way, so the
//! comparisons that follow them still hold. AshGraphql reports each redacted field as a
//! `forbidden_field` error as well as a null, which ash-graphql doesn't (see GAPS.md):
//! those errors are counted, and left out of the comparison.
//!
//! The same goes for AshTypescript's RPC (`POST /rpc/run`): the data each action answers,
//! and the types of the errors it fails with.

use std::process::ExitCode;
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

const ORG: &str = "acme";

/// One desk, and how it's reached.
struct Desk {
    name: &'static str,
    base: String,
    client: Client<HttpConnector, Full<Bytes>>,
}

/// Who a request runs as.
#[derive(Clone)]
struct As {
    role: &'static str,
    id: String,
}

impl Desk {
    fn new(name: &'static str, base: &str) -> Self {
        Self {
            name,
            base: base.trim_end_matches('/').to_string(),
            client: Client::builder(TokioExecutor::new()).build(HttpConnector::new()),
        }
    }

    /// `POST path` with `body`, as `who`: the status and the JSON it answered.
    async fn post(&self, path: &str, who: &As, body: Value) -> (u16, Value) {
        let request = hyper::Request::post(format!("{}{path}", self.base))
            .header("content-type", "application/json")
            .header("x-org", ORG)
            .header("x-actor", &who.id)
            .header("x-role", who.role)
            .body(Full::new(Bytes::from(body.to_string())))
            .expect("a request");
        let response = self.client.request(request).await.expect("the desk answers");
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.expect("a body").to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn graphql(&self, who: &As, query: &str, variables: Value) -> Value {
        self.post("/graphql", who, json!({ "query": query, "variables": variables })).await.1
    }
}

/// What's compared of a GraphQL response: its data, and its errors' codes but for
/// `forbidden_field`, which only AshGraphql reports.
fn comparable(response: &Value, forbidden: &mut usize) -> Value {
    let errors: Vec<Value> = response["errors"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|error| {
            let redacted = error["code"] == "forbidden_field";
            *forbidden += usize::from(redacted);
            !redacted
        })
        .map(|error| json!({ "path": error["path"] }))
        .collect();
    json!({ "data": strip_cursors(&response["data"]), "errors": errors })
}

/// What's compared of an RPC response: its data, but for keyset cursors, or the types of
/// its errors.
fn rpc_comparable(response: &Value) -> Value {
    if response["success"] == true {
        json!({ "data": strip_cursors(&response["data"]) })
    } else {
        let types: Vec<Value> = response["errors"].as_array().into_iter().flatten().map(|e| e["type"].clone()).collect();
        json!({ "errors": types })
    }
}

/// Keyset cursors encode the same position differently on each desk.
fn strip_cursors(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !matches!(key.as_str(), "startKeyset" | "endKeyset" | "nextPage" | "previousPage"))
                .map(|(key, value)| (key.clone(), strip_cursors(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(strip_cursors).collect()),
        other => other.clone(),
    }
}

/// The first place two values differ, for the report.
fn difference(path: &str, rust: &Value, elixir: &Value) -> Option<String> {
    match (rust, elixir) {
        (Value::Object(a), Value::Object(b)) => {
            let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
            keys.sort();
            keys.dedup();
            keys.into_iter().find_map(|key| {
                difference(&format!("{path}.{key}"), a.get(key).unwrap_or(&Value::Null), b.get(key).unwrap_or(&Value::Null))
            })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            a.iter().zip(b).enumerate().find_map(|(i, (a, b))| difference(&format!("{path}[{i}]"), a, b))
        }
        (a, b) if a == b => None,
        (a, b) => Some(format!("{path}: rust {} / elixir {}", short(a), short(b))),
    }
}

fn short(value: &Value) -> String {
    let text = value.to_string();
    if text.len() > 120 { format!("{}…", &text[..120]) } else { text }
}

struct Report {
    passed: usize,
    failed: Vec<String>,
    forbidden: usize,
}

impl Report {
    fn check(&mut self, name: &str, rust: &Value, elixir: &Value) {
        match difference("", rust, elixir) {
            None => {
                self.passed += 1;
                println!("  ok    {name}");
            }
            Some(diff) => {
                println!("  DIFF  {name}\n          {diff}");
                self.failed.push(name.to_string());
            }
        }
    }
}

const INBOX: &str = "query Inbox($after: String) {
  listTickets(first: 25, after: $after, sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }]) {
    count endKeyset results { id subject status priority confidential requesterEmail assigneeId }
  }
}";

const DASHBOARD: &str = "{
  listTickets(
    first: 25,
    filter: { commentCount: { greaterThanOrEqual: 5 }, weight: { greaterThanOrEqual: 30 } },
    sort: [{ field: COMMENT_COUNT, order: DESC }, { field: INSERTED_AT, order: DESC }, { field: ID }]
  ) { count results { id commentCount publicCommentCount hasInternalNotes weight subjectLength } }
}";

const DETAIL: &str = "{
  listTickets(first: 50, sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }]) {
    results {
      id author { name } assignee { name }
      tags(sort: [{ field: NAME }]) { name }
      comments(sort: [{ field: INSERTED_AT, order: DESC }, { field: ID }], limit: 3) { body internal }
    }
  }
}";

const AGENTS: &str = "{ listAgents(sort: [{ field: NAME }]) { results { name role active openAssigned } } }";

#[tokio::main]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (Some(rust_url), Some(elixir_url), Some(fixture_path)) = (flag("--rust"), flag("--elixir"), flag("--fixture")) else {
        eprintln!("usage: parity --rust URL --elixir URL --fixture fixture.json");
        return ExitCode::FAILURE;
    };
    let fixture: Value = serde_json::from_slice(&std::fs::read(&fixture_path).expect("the fixture")).expect("JSON");
    let staff: Vec<&Value> = fixture["agents"].as_array().unwrap().iter().filter(|a| a["org"] == ORG).collect();
    let actor = |role: &'static str| As {
        role,
        id: staff.iter().find(|a| a["role"] == role).unwrap()["id"].as_str().unwrap().to_string(),
    };
    let (admin, agent, viewer) = (actor("admin"), actor("agent"), actor("viewer"));
    // A ticket the agent is assigned, for the field policy and the writes.
    let assigned = fixture["tickets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["org"] == ORG && t["assignee_id"] == agent.id.as_str() && t["status"] == "new" && t["confidential"] == false)
        .expect("a new ticket assigned to the agent")["id"]
        .as_str()
        .unwrap()
        .to_string();

    let tickets: Vec<&Value> = fixture["tickets"].as_array().unwrap().iter().filter(|t| t["org"] == ORG).collect();
    let ticket = |what: &str, pick: &dyn Fn(&Value) -> bool| {
        tickets.iter().find(|t| t["id"] != assigned.as_str() && pick(t)).unwrap_or_else(|| panic!("{what}"))["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let commented = |t: &Value| fixture["comments"].as_array().unwrap().iter().any(|c| c["ticket_id"] == t["id"]);
    // An open ticket with comments, for RPC's reads and writes; a confidential new one,
    // which a viewer can't read; and a new one anyone reads.
    let open = ticket("an open ticket with comments", &|t| t["status"] == "open" && t["confidential"] == false && commented(t));
    let secret = ticket("a confidential new ticket", &|t| t["status"] == "new" && t["confidential"] == true);
    let plain = ticket("a new ticket", &|t| t["status"] == "new" && t["confidential"] == false);

    let desks = [Desk::new("rust", &rust_url), Desk::new("elixir", &elixir_url)];
    let mut report = Report { passed: 0, failed: Vec::new(), forbidden: 0 };
    let both = |query: &'static str, who: As, variables: Value| {
        let desks = &desks;
        async move {
            let r = desks[0].graphql(&who, query, variables.clone()).await;
            let e = desks[1].graphql(&who, query, variables).await;
            (r, e)
        }
    };

    println!("Reads, as each role:");
    for who in [&admin, &agent, &viewer] {
        for (name, query) in [("inbox", INBOX), ("dashboard", DASHBOARD), ("detail graph", DETAIL), ("agents", AGENTS)] {
            let (r, e) = both(query, who.clone(), json!({})).await;
            let (r, e) = (comparable(&r, &mut report.forbidden), comparable(&e, &mut report.forbidden));
            report.check(&format!("{name} as {}", who.role), &r, &e);
        }
        // The second page, each desk from its own cursor.
        let (r, e) = both(INBOX, who.clone(), json!({})).await;
        let next = |page: &Value| json!({ "after": page["data"]["listTickets"]["endKeyset"] });
        let r2 = desks[0].graphql(who, INBOX, next(&r)).await;
        let e2 = desks[1].graphql(who, INBOX, next(&e)).await;
        let (r2, e2) = (comparable(&r2, &mut report.forbidden), comparable(&e2, &mut report.forbidden));
        report.check(&format!("inbox page 2 as {}", who.role), &r2, &e2);
    }

    println!("Writes:");
    let ticket_q = "query T($id: ID!) { getTicket(id: $id) { status viewCount version requesterEmail assignee { name } } }";
    let id = json!({ "id": assigned });
    let start = "mutation M($id: ID!) { startTicket(id: $id) { result { status } errors { code } } }";
    // Whether to compare the errors' codes too: the desks code unexpected errors apart.
    for (name, mutation, who, id, codes) in [
        ("view as agent", "mutation M($id: ID!) { viewTicket(id: $id) { result { viewCount version } errors { code } } }", &agent, &id, false),
        ("start as agent", start, &agent, &id, false),
        ("close an open ticket", "mutation M($id: ID!) { closeTicket(id: $id) { result { status } errors { code } } }", &agent, &id, false),
        ("resolve as viewer", "mutation M($id: ID!) { resolveTicket(id: $id) { result { status } errors { code } } }", &viewer, &id, false),
        ("resolve as agent", "mutation M($id: ID!) { resolveTicket(id: $id) { result { status } errors { code } } }", &agent, &id, false),
        // Found through the read, whose policies hide it from a viewer: not found, not forbidden.
        ("start a ticket the viewer can't read", start, &viewer, &json!({ "id": secret }), true),
    ] {
        let (r, e) = both(mutation, who.clone(), id.clone()).await;
        let outcome = |v: &Value| {
            let result = v["data"].as_object().and_then(|d| d.values().next()).cloned().unwrap_or(Value::Null);
            let failed = !result["errors"].as_array().is_none_or(Vec::is_empty) || result["result"].is_null();
            let codes = if codes { result["errors"].clone() } else { Value::Null };
            json!({ "result": if failed { Value::Null } else { result["result"].clone() }, "failed": failed, "codes": codes })
        };
        report.check(name, &outcome(&r), &outcome(&e));
    }
    let (r, e) = both(ticket_q, agent.clone(), id.clone()).await;
    let (r, e) = (comparable(&r, &mut report.forbidden), comparable(&e, &mut report.forbidden));
    report.check("the ticket after the writes, as its assignee", &r, &e);

    println!("RPC:");
    let nested_comments = json!({ "comments": { "fields": ["body", "internal"], "sort": "insertedAt,id", "filter": { "internal": { "eq": false } } } });
    for (name, who, body) in [
        ("list, filtered and sorted", &agent, json!({ "action": "list_tickets", "fields": ["id", "subject", "requesterEmail", "commentCount", "weight", { "assignee": ["name"] }], "sort": "-insertedAt,id", "filter": { "priority": { "greaterThanOrEqual": 4 } } })),
        ("a keyset page, counted", &viewer, json!({ "action": "list_tickets", "fields": ["id", "subject"], "sort": "-insertedAt,id", "page": { "limit": 3, "count": true } })),
        ("an offset page", &agent, json!({ "action": "list_tickets", "fields": ["id"], "sort": "-insertedAt,id", "page": { "limit": 2, "offset": 4 } })),
        ("get, with relationships", &admin, json!({ "action": "get_ticket", "getBy": { "id": open }, "fields": ["id", "status", "requesterEmail", "weight", nested_comments, { "tags": { "fields": ["name"], "sort": "name" } }] })),
        ("get, as a viewer", &viewer, json!({ "action": "get_ticket", "getBy": { "id": open }, "fields": ["id", { "comments": ["internal"] }] })),
        ("get, missing", &agent, json!({ "action": "get_ticket", "getBy": { "id": "00000000-0000-0000-0000-000000000000" }, "fields": ["id"] })),
        // As a viewer: AshTypescript loads relationships without the actor (see GAPS.md),
        // so an agent's nested internal notes are hidden there.
        ("nested limit and sort", &viewer, json!({ "action": "list_tickets", "fields": ["id", { "comments": { "fields": ["body"], "limit": 2, "sort": "-insertedAt,id" } }], "sort": "-insertedAt,id", "page": { "limit": 2 } })),
        ("nested filter", &agent, json!({ "action": "list_tickets", "fields": ["id", { "comments": { "fields": ["internal"], "filter": { "internal": { "eq": false } } } }], "sort": "-insertedAt,id", "page": { "limit": 2, "offset": 0 } })),
        ("agents", &agent, json!({ "action": "list_agents", "fields": ["name", "openAssigned"], "sort": "name", "filter": { "role": { "eq": "agent" } } })),
        ("view", &agent, json!({ "action": "view_ticket", "identity": open, "fields": ["viewCount", "version"] })),
        ("assign, selecting nothing", &agent, json!({ "action": "assign_ticket", "identity": open, "input": { "assigneeId": agent.id } })),
        ("start, as a viewer who can't read it", &viewer, json!({ "action": "start_ticket", "identity": secret, "fields": ["status"] })),
        ("start, as a viewer", &viewer, json!({ "action": "start_ticket", "identity": plain, "fields": ["status"] })),
        ("start", &agent, json!({ "action": "start_ticket", "identity": secret, "fields": ["status"] })),
        ("close an open ticket", &agent, json!({ "action": "close_ticket", "identity": open, "fields": ["status"] })),
        ("open, invalid", &agent, json!({ "action": "open_ticket", "input": { "subject": "Hello there", "body": "b", "priority": 9, "requesterEmail": "a@b.c" }, "fields": ["id"] })),
        ("open, with comments", &agent, json!({ "action": "open_ticket", "input": { "subject": "Hello there", "body": "b", "priority": 2, "requesterEmail": "a@b.c", "comments": [{ "body": "first" }] }, "fields": ["subject", "status", "requesterEmail", "commentCount", { "comments": ["body"] }, { "author": ["name"] }] })),
        ("create a comment", &agent, json!({ "action": "create_comment", "input": { "ticketId": open, "body": "via rpc" }, "fields": ["body", "internal", "authorId"] })),
        ("route", &agent, json!({ "action": "route_ticket", "input": { "subject": "Printer on fire", "body": "Smoke", "priority": 3, "requesterEmail": "pat@example.com" } })),
        ("an unknown action", &agent, json!({ "action": "nope", "fields": ["id"] })),
        ("an unknown field", &agent, json!({ "action": "list_tickets", "fields": ["id", "nope"] })),
        ("a relationship without fields", &agent, json!({ "action": "list_tickets", "fields": ["id", "assignee"] })),
    ] {
        let r = desks[0].post("/rpc/run", who, body.clone()).await.1;
        let e = desks[1].post("/rpc/run", who, body).await.1;
        // A new record's id is each desk's own.
        let comparable = |v: &Value| match &v["data"] {
            Value::String(id) if uuid::Uuid::parse_str(id).is_ok() => json!({ "data": "an id" }),
            _ => rpc_comparable(v),
        };
        report.check(name, &comparable(&r), &comparable(&e));
    }

    println!("JSON endpoints:");
    let route = json!({
        "subject": "Printer on fire", "body": "Smoke everywhere", "priority": 4, "requesterEmail": "pat@example.com",
        "comments": [{ "body": "Called it in" }, { "body": "Escalate", "internal": true }],
    });
    let mut routed = Vec::new();
    for desk in &desks {
        let (status, body) = desk.post("/api/route", &agent, route.clone()).await;
        routed.push((status, body["id"].as_str().unwrap_or_default().to_string()));
    }
    report.check("route: status", &json!(routed[0].0), &json!(routed[1].0));
    let routed_q = "query T($id: ID!) { getTicket(id: $id) { subject status assignee { name } comments(sort: [{ field: BODY }]) { body internal } commentCount } }";
    let r = desks[0].graphql(&admin, routed_q, json!({ "id": routed[0].1 })).await;
    let e = desks[1].graphql(&admin, routed_q, json!({ "id": routed[1].1 })).await;
    let (r, e) = (comparable(&r, &mut report.forbidden), comparable(&e, &mut report.forbidden));
    report.check("route: the ticket it opened", &r, &e);
    for (name, path, who, body) in [
        ("route as viewer", "/api/route", &viewer, route.clone()),
        ("route an invalid ticket", "/api/route", &agent, json!({ "subject": "x", "body": "y", "priority": 9, "requesterEmail": "a@b.c" })),
        ("bulk", "/api/bulk", &agent, json!({ "count": 100, "assigneeId": agent.id })),
        ("edit at a stale version", "/api/edit", &agent, json!({ "id": assigned, "version": 999, "subject": "Stale edit" })),
    ] {
        let (rs, rb) = desks[0].post(path, who, body.clone()).await;
        let (es, eb) = desks[1].post(path, who, body).await;
        let keep = |status: u16, body: &Value| {
            if status == 200 { json!({ "status": status, "body": body }) } else { json!({ "status": status, "error": body["error"] }) }
        };
        report.check(name, &keep(rs, &rb), &keep(es, &eb));
    }

    println!("Subscriptions:");
    let mut heard = Vec::new();
    for desk in &desks {
        heard.push(listen_once(desk, &agent, &assigned).await);
    }
    report.check("ticketUpdated, heard by the assignee", &heard[0], &heard[1]);

    println!(
        "\n{} matched, {} differed; AshGraphql also reported {} redacted fields as forbidden_field errors",
        report.passed,
        report.failed.len(),
        report.forbidden
    );
    if report.failed.is_empty() { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}

/// Subscribes to `ticketUpdated` as `who`, views `ticket`, and returns what was heard.
async fn listen_once(desk: &Desk, who: &As, ticket: &str) -> Value {
    let mut request = format!("{}/graphql/ws", desk.base.replacen("http", "ws", 1)).into_client_request().unwrap();
    let headers = request.headers_mut();
    headers.insert("Sec-WebSocket-Protocol", "graphql-transport-ws".parse().unwrap());
    headers.insert("x-org", ORG.parse().unwrap());
    headers.insert("x-actor", who.id.parse().unwrap());
    headers.insert("x-role", who.role.parse().unwrap());
    let Ok((mut socket, _)) = tokio_tungstenite::connect_async(request).await else {
        return json!(format!("{} refused the socket", desk.name));
    };
    let send = |value: Value| Message::Text(value.to_string().into());
    socket.send(send(json!({ "type": "connection_init", "payload": {} }))).await.unwrap();
    // Subscribe once the server has accepted the connection, as the protocol has it.
    let acked = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match tokio::time::timeout_at(acked, socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) if text.contains("connection_ack") => break,
            Ok(Some(Ok(_))) => {}
            _ => return json!(format!("{} never acknowledged the socket", desk.name)),
        }
    }
    let query = "subscription { ticketUpdated { updated { id viewCount } } }";
    socket.send(send(json!({ "id": "1", "type": "subscribe", "payload": { "query": query } }))).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let view = "mutation M($id: ID!) { viewTicket(id: $id) { result { viewCount } } }";
    desk.graphql(who, view, json!({ "id": ticket })).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while let Ok(Some(Ok(message))) = tokio::time::timeout_at(deadline, socket.next()).await {
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            if value["type"] == "next" {
                return value["payload"]["data"]["ticketUpdated"]["updated"].clone();
            }
        }
    }
    json!("nothing heard")
}
