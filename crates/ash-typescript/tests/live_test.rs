//! Live data in generated clients: the SDK type-checks with its React hooks, and a
//! generated client run in Node keeps live queries in sync with a real `ash-graphql`
//! server over WebSocket.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use ash_core::{ActionDef, AttrType, AttributeDef, Context, DomainDef, ResourceDef};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::{ContextPubSubExt, PubSub};
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open").accept(&["title", "priority"]),
    ActionDef::update("reprioritize").accept(&["priority"]),
    ActionDef::destroy("remove"),
    ActionDef::read("read").primary(),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("title", AttrType::String),
        AttributeDef::required("priority", AttrType::Integer),
    ],
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

static DOMAIN: DomainDef = DomainDef {
    name: "Desk",
    resources: &[&TICKET_DEF],
};

fn sdk(react: bool) -> String {
    TypeScriptGenerator::new()
        .config(
            TypeScriptConfig::new()
                .with_zod(false)
                .with_client(true)
                .with_react(react)
                .with_subscriptions(true)
                .with_client_name("DeskClient"),
        )
        .add_domain(&DOMAIN)
        .generate_consolidated()
        .unwrap()
}

/// Runs `program` with `args`, or returns `None` when it isn't installed outside CI.
fn run(program: &str, args: &[&str], dir: &Path) -> Option<std::process::Output> {
    match Command::new(program).args(args).current_dir(dir).output() {
        Ok(output) => Some(output),
        Err(_) => {
            assert!(std::env::var("CI").is_err(), "CI installs {program}");
            eprintln!("skipping: {program} is not installed");
            None
        }
    }
}

fn assert_success(output: &std::process::Output, what: &str) {
    assert!(
        output.status.success(),
        "{what} failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn live_sdk_without_live_data_is_unchanged() {
    let plain = TypeScriptGenerator::new()
        .config(TypeScriptConfig::new().with_zod(false))
        .add_domain(&DOMAIN)
        .generate_consolidated()
        .unwrap();
    assert!(!plain.contains("AshSubscriptionClient"));
    assert!(!plain.contains("from \"react\""));

    let live = sdk(true);
    for expected in [
        "export class AshSubscriptionClient",
        "export function ashLiveQuery",
        "public onCreated(",
        "public live(",
        "export function useTicketLive(",
        "export function useAshConnectionStatus(",
        "import { useEffect, useState } from \"react\";",
    ] {
        assert!(live.contains(expected), "missing {expected}");
    }
}

#[test]
fn live_sdk_type_checks_with_react_hooks() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sdk.ts"), sdk(true)).unwrap();
    // Just enough of React's types for the hooks.
    let react = dir.path().join("node_modules/react");
    std::fs::create_dir_all(&react).unwrap();
    std::fs::write(
        react.join("index.d.ts"),
        "export function useState<S>(initial: S | (() => S)): [S, (next: S | ((current: S) => S)) => void];\n\
         export function useEffect(effect: () => void | (() => void), deps?: readonly unknown[]): void;\n",
    )
    .unwrap();
    let args = [
        "--noEmit", "--strict", "--target", "ES2022", "--moduleResolution", "node",
        "--skipLibCheck", "sdk.ts",
    ];
    if let Some(output) = run("tsc", &args, dir.path()) {
        assert_success(&output, "tsc");
    }
}

const PROGRAM: &str = r#"
import { DeskClient, type Ticket } from "./sdk";

declare const process: { argv: string[]; exit(code: number): never };

const client = new DeskClient({ baseUrl: process.argv[2] });
const statuses: string[] = [];
client.subscriptions.onStatus((status) => statuses.push(status));
const errors: unknown[] = [];
const onError = (error: unknown) => errors.push(error);

function until(label: string, read: () => Ticket[], test: (items: Ticket[]) => boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const poll = () => {
      if (test(read())) return resolve();
      if (Date.now() - started > 8000) return reject(new Error(`timed out waiting for ${label}: ${JSON.stringify(read())} statuses=${statuses.join(",")}`));
      setTimeout(poll, 20);
    };
    poll();
  });
}

async function main() {
  // Every ticket, kept exact by events alone.
  let all: Ticket[] = [];
  const everything = client.ticket.query().live((items) => (all = items), { onError });
  // Urgent tickets, most urgent first, re-read after changes.
  let urgent: Ticket[] = [];
  const triage = client.ticket
    .query()
    .filter({ priority: { gte: 5 } })
    .sort("priority", "desc")
    .live((items) => (urgent = items), { syncDelayMs: 50, onError });

  await until("the seeded ticket", () => all, (items) => items.length === 1);
  const minor = await client.ticket.open({ title: "Minor", priority: 1 });
  await until("the new ticket", () => all, (items) => items.length === 2);

  await client.ticket.reprioritize(minor.id, { priority: 9 });
  await until("the update", () => all, (items) => items.some((t) => t.id === minor.id && t.priority === 9));
  await until("it to become urgent", () => urgent, (items) => items[0]?.id === minor.id);

  await client.ticket.reprioritize(minor.id, { priority: 2 });
  await until("it to leave the urgent list", () => urgent, (items) => !items.some((t) => t.id === minor.id));

  await client.ticket.remove(minor.id);
  await until("the removal", () => all, (items) => items.length === 1);

  everything.stop();
  triage.stop();
  client.subscriptions.close();
  if (errors.length > 0) throw new Error(`live errors: ${errors.map(String).join("; ")}`);
  console.log(`OK ${statuses.join(",")}`);
}

main().then(
  () => process.exit(0),
  (error) => {
    console.error(error);
    process.exit(1);
  },
);
"#;

#[tokio::test(flavor = "multi_thread")]
async fn generated_live_queries_stay_in_sync_over_websocket() {
    let pubsub = PubSub::new();
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let mut seed = ash_core::FieldMap::new();
    seed.insert("title".into(), ash_core::Value::String("Seeded".into()));
    seed.insert("priority".into(), ash_core::Value::Int(3));
    ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], seed).await.unwrap();
    let schema = AshGraphQL::from_domain(&DOMAIN)
        .with_pubsub(pubsub)
        .finish_with_context(ctx)
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, ash_graphql::axum::graphql_router(schema)).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sdk.ts"), sdk(false)).unwrap();
    std::fs::write(dir.path().join("main.ts"), PROGRAM).unwrap();
    let compile = [
        "--strict", "--target", "ES2022", "--module", "commonjs", "--skipLibCheck",
        "--outDir", "out", "sdk.ts", "main.ts",
    ];
    let Some(compiled) = run("tsc", &compile, dir.path()) else { return };
    assert_success(&compiled, "tsc");

    let base_url = format!("http://{addr}");
    let dir_path = dir.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        run("node", &["out/main.js", &base_url], &dir_path)
    })
    .await
    .unwrap();
    let Some(output) = output else { return };
    assert_success(&output, "the live client");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("OK idle,connecting,connected"), "{stdout}");
}
