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

// Counts the client's reads: requests that aren't mutations.
let reads = 0;
const realFetch = globalThis.fetch;
globalThis.fetch = (input: RequestInfo | URL, init?: RequestInit) => {
  if (!String(init?.body ?? "").includes("mutation")) reads += 1;
  return realFetch(input, init);
};

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
    .filter({ priority: { greaterThanOrEqual: 5 } })
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

  // A filtered, sorted list the client can evaluate stays exact without re-reading:
  // records enter it, move within it and leave it where the server would put them.
  const readsBefore = reads;
  const seven = await client.ticket.open({ title: "Seven", priority: 7 });
  const six = await client.ticket.open({ title: "Six", priority: 6 });
  await until("both urgent, in order", () => urgent, (items) => items.map((t) => t.title).join() === "Seven,Six");
  await client.ticket.reprioritize(six.id, { priority: 8 });
  await until("six to move up", () => urgent, (items) => items.map((t) => t.title).join() === "Six,Seven");
  await client.ticket.reprioritize(seven.id, { priority: 1 });
  await until("seven to leave", () => urgent, (items) => items.map((t) => t.title).join() === "Six");
  await new Promise((resolve) => setTimeout(resolve, 200));
  if (reads !== readsBefore) throw new Error(`the urgent list re-read ${reads - readsBefore} times`);
  await client.ticket.remove(six.id);
  await client.ticket.remove(seven.id);
  await until("the cleanup", () => all, (items) => items.length === 1);

  // A burst of changes reaches the listener in a few notifications, not one apiece.
  let heard = 0;
  let burst: Ticket[] = [];
  const counted = client.ticket.query().live((items) => {
    heard += 1;
    burst = items;
  }, { onError });
  await until("the counted list", () => burst, (items) => items.length === 1);
  const before = heard;
  await Promise.all(Array.from({ length: 40 }, (_, i) => client.ticket.open({ title: `Burst ${i}`, priority: 1 })));
  await until("the burst", () => burst, (items) => items.length === 41);
  if (heard - before > 20) throw new Error(`heard ${heard - before} notifications for 40 creates`);
  counted.stop();

  // Every read pages, so all() follows the keysets past the first page.
  await Promise.all(Array.from({ length: 220 }, (_, i) => client.ticket.open({ title: `More ${i}`, priority: 2 })));
  const everyTicket = await client.ticket.query().all();
  const someTickets = await client.ticket.query().limit(255).all();
  if (everyTicket.length !== 261 || new Set(everyTicket.map((t) => t.id)).size !== 261 || someTickets.length !== 255) {
    throw new Error(`all() read ${everyTicket.length}, limit(255) ${someTickets.length}`);
  }
  await Promise.all(everyTicket.filter((t) => t.title.startsWith("More")).map((t) => client.ticket.remove(t.id)));

  // Keyset pages walk the list in order, and say how many records there are.
  const pages = client.ticket.query().sort("title", "asc");
  const first = await pages.page(30);
  const rest = await pages.page(30, first.endKeyset ?? undefined);
  if (first.count !== 41 || first.results.length !== 30 || rest.results.length !== 11) {
    throw new Error(`paged ${first.results.length} + ${rest.results.length} of ${first.count}`);
  }
  const back = await pages.page(30, undefined, rest.startKeyset ?? undefined);
  if (back.results.map((t) => t.id).join() !== first.results.map((t) => t.id).join()) {
    throw new Error("paging back didn't return the first page");
  }

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

const CATCH_UP: &str = r#"
import { DeskClient, type Ticket } from "./sdk";

declare const process: {
  argv: string[];
  exit(code: number): never;
  stdout: { write(text: string): void };
};

const client = new DeskClient({ baseUrl: process.argv[2] });
const burst = Number(process.argv[3]);
const errors: unknown[] = [];
const onError = (error: unknown) => errors.push(error);

function until(label: string, test: () => boolean): Promise<void> {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    const poll = () => {
      if (test()) return resolve();
      if (Date.now() - started > 10000) return reject(new Error(`timed out waiting for ${label}`));
      setTimeout(poll, 20);
    };
    poll();
  });
}

async function main() {
  let all: Ticket[] = [];
  const everything = client.ticket.query().live((items) => (all = items), { onError });
  const heard: string[] = [];
  const missed: Array<number | undefined> = [];
  const stopListening = client.ticket.onCreated((ticket) => heard.push(ticket.title), {
    onError,
    onMissed: (count) => missed.push(count),
  });

  // Live: a ticket of our own comes back over the subscription.
  await until("the seeded ticket", () => all.length === 1);
  await client.ticket.open({ title: "Probe", priority: 1 });
  await until("the probe", () => heard.includes("Probe"));
  process.stdout.write("READY\n");

  // The server bursts far more creates than this subscriber's buffer holds.
  await until("the burst, re-read", () => all.length === 2 + burst);
  // The server said how many it missed.
  await until("the missed events", () => missed.some((count) => count !== undefined && count > 0));
  // Resubscribed: a create after the burst is heard, and lands in the list.
  await client.ticket.open({ title: "After", priority: 1 });
  await until("the create after", () => heard.includes("After") && all.length === 3 + burst);

  everything.stop();
  stopListening();
  client.subscriptions.close();
  if (errors.length > 0) throw new Error(`live errors: ${errors.map(String).join("; ")}`);
  console.log(`OK missed=${missed.join(",")}`);
}

main().then(
  () => process.exit(0),
  (error) => {
    console.error(error);
    process.exit(1);
  },
);
"#;

/// A live query whose subscriber falls behind the server re-reads and catches up, and its
/// subscriptions carry on. The server runs on one thread, so a burst of creates runs
/// without its subscriptions draining a one-event buffer: they're certain to fall behind.
#[tokio::test(flavor = "current_thread")]
async fn a_live_query_that_falls_behind_catches_up() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};

    let pubsub = PubSub::with_capacity(1);
    let ctx = Context::new(Memory::new()).with_pubsub(Arc::new(pubsub.clone()));
    let open = |title: String| {
        let ctx = ctx.clone();
        async move {
            let mut input = ash_core::FieldMap::new();
            input.insert("title".into(), ash_core::Value::String(title));
            input.insert("priority".into(), ash_core::Value::Int(3));
            ash_core::create_dynamic(&ctx, &TICKET_DEF, &TICKET_ACTIONS[0], input).await.unwrap();
        }
    };
    open("Seeded".into()).await;
    let schema = AshGraphQL::from_domain(&DOMAIN)
        .with_pubsub(pubsub)
        .finish_with_context(ctx.clone())
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, ash_graphql::axum::graphql_router(schema)).await.unwrap();
    });

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sdk.ts"), sdk(false)).unwrap();
    std::fs::write(dir.path().join("main.ts"), CATCH_UP).unwrap();
    let compile = [
        "--strict", "--target", "ES2022", "--module", "commonjs", "--skipLibCheck",
        "--outDir", "out", "sdk.ts", "main.ts",
    ];
    let Some(compiled) = run("tsc", &compile, dir.path()) else { return };
    assert_success(&compiled, "tsc");

    const BURST: usize = 300;
    let mut client = match tokio::process::Command::new("node")
        .args(["out/main.js", &format!("http://{addr}"), &BURST.to_string()])
        .current_dir(dir.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(client) => client,
        Err(_) => {
            assert!(std::env::var("CI").is_err(), "CI installs node");
            return;
        }
    };
    let mut stdout = BufReader::new(client.stdout.take().unwrap()).lines();
    let mut stderr = client.stderr.take().unwrap();
    let ready = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        while let Some(line) = stdout.next_line().await.unwrap() {
            if line == "READY" {
                return true;
            }
        }
        false
    })
    .await;
    if ready != Ok(true) {
        let mut errors = String::new();
        stderr.read_to_string(&mut errors).await.unwrap();
        panic!("the client never got ready: {errors}");
    }

    for i in 0..BURST {
        open(format!("Burst {i}")).await;
    }

    let status = tokio::time::timeout(std::time::Duration::from_secs(30), client.wait())
        .await
        .expect("the client finishes")
        .unwrap();
    let mut out = Vec::new();
    while let Some(line) = stdout.next_line().await.unwrap() {
        out.push(line);
    }
    let mut errors = String::new();
    stderr.read_to_string(&mut errors).await.unwrap();
    assert!(status.success(), "the live client failed:\n{}\n{errors}", out.join("\n"));
    assert!(out.iter().any(|line| line.starts_with("OK missed=")), "{out:?}");
}
