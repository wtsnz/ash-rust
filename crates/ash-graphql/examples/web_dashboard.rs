//! Interactive Web Dashboard Example for ash-rust.
//!
//! Run with:
//! ```bash
//! cargo run -p ash-graphql --example web_dashboard --features axum
//! ```
//!
//! Then open:
//! - Web Dashboard & Overview: http://127.0.0.1:4000/
//! - GraphiQL Interactive IDE: http://127.0.0.1:4000/graphiql

use std::collections::HashMap;
use ash_core::{
    ActionDef, AttrType, AttributeDef, Context, DataLayer, FieldMap, ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::PubSub;
use axum::response::Html;
use axum::routing::get;
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["OPEN", "IN_PROGRESS", "CLOSED"],
        },
    ),
    AttributeDef::required("priority", AttrType::Integer),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("open")
        .primary()
        .accept(&["title", "status", "priority"]),
    ActionDef::update("change_status")
        .primary()
        .accept(&["status"]),
    ActionDef::destroy("close").primary(),
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static REP_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::required("email", AttrType::String),
];

static REP_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("create")
        .primary()
        .accept(&["name", "email"]),
];

static REP_DEF: ResourceDef = ResourceDef {
    name: "Representative",
    table: "representatives",
    attributes: REP_ATTRS,
    relationships: &[],
    actions: REP_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn dashboard_landing() -> Html<&'static str> {
    Html(r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>ash-rust Web Dashboard</title>
  <style>
    body {
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      max-width: 900px;
      margin: 40px auto;
      padding: 0 20px;
      line-height: 1.6;
      color: #24292f;
      background: #f6f8fa;
    }
    .card {
      background: white;
      border: 1px solid #d0d7de;
      border-radius: 8px;
      padding: 24px;
      margin-bottom: 24px;
      box-shadow: 0 1px 3px rgba(0,0,0,0.05);
    }
    h1 { color: #0969da; margin-top: 0; }
    h2 { border-bottom: 1px solid #eaecef; padding-bottom: 8px; }
    .badge {
      display: inline-block;
      padding: 4px 8px;
      font-size: 12px;
      font-weight: 600;
      border-radius: 12px;
      background: #ddf4ff;
      color: #0969da;
    }
    .btn {
      display: inline-block;
      background: #1f883d;
      color: white;
      text-decoration: none;
      padding: 10px 18px;
      font-weight: 600;
      border-radius: 6px;
      margin-top: 10px;
    }
    .btn:hover { background: #1a7f37; }
    pre {
      background: #f6f8fa;
      border: 1px solid #d0d7de;
      padding: 12px;
      border-radius: 6px;
      overflow-x: auto;
      font-size: 14px;
    }
    code {
      background: #eff1f3;
      padding: 2px 6px;
      border-radius: 4px;
      font-size: 13px;
    }
  </style>
</head>
<body>
  <div class="card">
    <span class="badge">ash-rust</span>
    <h1>Declarative Ash Web Dashboard</h1>
    <p>Welcome to the <strong>ash-rust</strong> web dashboard! Your Ash domain schema is automatically reflected into a live GraphQL schema and interactive GraphiQL IDE with full documentation, queries, mutations, and Relay cursor pagination.</p>
    <a class="btn" href="/graphiql">Open Interactive GraphiQL IDE &rarr;</a>
  </div>

  <div class="card">
    <h2>Registered Domain Resources</h2>
    <ul>
      <li><strong>Ticket</strong> (attributes: <code>id</code>, <code>title</code>, <code>status</code>, <code>priority</code>)</li>
      <li><strong>Representative</strong> (attributes: <code>id</code>, <code>name</code>, <code>email</code>)</li>
    </ul>
  </div>

  <div class="card">
    <h2>Try Queries in GraphiQL</h2>
    <p>Open <a href="/graphiql">/graphiql</a> and try running:</p>
    <pre>
# 1. List all seeded tickets
query {
  listTickets {
    id
    title
    status
    priority
  }
}

# 2. Open a new ticket mutation
mutation {
  openTicket(input: { title: "WiFi dropped in conference room B", status: OPEN, priority: 1 }) {
    result {
      id
      title
      status
    }
    errors {
      field
      message
    }
  }
}

# 3. Relay connection with cursor pagination
query {
  ticketConnection(first: 2) {
    totalCount
    pageInfo {
      hasNextPage
      endCursor
    }
    edges {
      cursor
      node {
        id
        title
      }
    }
  }
}
    </pre>
  </div>
</body>
</html>"#)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mem = Memory::new();
    let pubsub = PubSub::new();

    // Seed initial tickets
    let id1 = Uuid::new_v4();
    let mut f1 = FieldMap::new();
    f1.insert("id".into(), Value::Uuid(id1));
    f1.insert("title".into(), Value::String("Network printer unreachable on 3rd floor".into()));
    f1.insert("status".into(), Value::String("OPEN".into()));
    f1.insert("priority".into(), Value::Int(2));
    mem.create(&TICKET_DEF, id1, f1).await?;

    let id2 = Uuid::new_v4();
    let mut f2 = FieldMap::new();
    f2.insert("id".into(), Value::Uuid(id2));
    f2.insert("title".into(), Value::String("Postgres replication lag investigation".into()));
    f2.insert("status".into(), Value::String("IN_PROGRESS".into()));
    f2.insert("priority".into(), Value::Int(1));
    mem.create(&TICKET_DEF, id2, f2).await?;

    let rep_id = Uuid::new_v4();
    let mut f_rep = HashMap::new();
    f_rep.insert("id".into(), Value::Uuid(rep_id));
    f_rep.insert("name".into(), Value::String("Alex Mercer".into()));
    f_rep.insert("email".into(), Value::String("alex@example.com".into()));
    mem.create(&REP_DEF, rep_id, f_rep).await?;

    let ctx = Context::new(mem);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF, &REP_DEF])
        .with_pubsub(pubsub)
        .with_dataloader()
        .finish_with_context(ctx)?;

    let app = ash_graphql::axum::graphql_router(schema)
        .route("/", get(dashboard_landing));

    let addr = "127.0.0.1:4000";
    let listener = tokio::net::TcpListener::bind(addr).await?;

    println!("\n========================================================");
    println!("  🚀 ash-rust Web Dashboard is running!");
    println!("========================================================");
    println!("  Overview & Schema: http://{}/", addr);
    println!("  GraphiQL IDE:      http://{}/graphiql", addr);
    println!("  GraphQL API:       http://{}/graphql", addr);
    println!("========================================================\n");

    axum::serve(listener, app).await?;
    Ok(())
}
