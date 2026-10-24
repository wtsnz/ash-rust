//! RPC write results and server-owned tenancy.
//! Each test asserts the secure behaviour. The control is the
//! same user's ordinary RPC read.

use ash_core::{Actor, Context, Value};
use ash_memory::Memory;
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use uuid::Uuid;

mod ledger {
    use ash_core::resource;
    use uuid::Uuid;

    use super::entry::Entry;

    resource! {
        Ledger {
            table "subrpc_ledgers";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                name: String;
                secret: Option<String>;
                amount: i64 [default: 0];
            }

            relationships {
                has_many entries: Entry [fk: ledger_id];
            }

            aggregates {
                entry_count: Option<i64> = count(entries);
                first_memo: Option<String> = first(entries, memo);
            }

            calculations {
                doubled: i64 = amount * 2;
            }

            policies {
                policy always { authorize_if always; }
            }

            field_policies {
                field secret { authorize_if actor_attribute_equals(role, "admin"); }
            }

            actions {
                read read { primary; }
                create create { primary; accept [id, name, secret, amount]; }
                update rename { primary; accept [name]; }
                destroy destroy { primary; }
            }
        }
    }
}

mod entry {
    use ash_core::resource;
    use uuid::Uuid;

    use super::ledger::Ledger;

    resource! {
        Entry {
            table "subrpc_entries";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                ledger_id: Uuid;
                memo: Option<String>;
            }

            field_policies {
                field memo { authorize_if actor_attribute_equals(role, "admin"); }
            }

            relationships {
                belongs_to ledger: Ledger [fk: ledger_id];
            }

            actions {
                read read { primary; }
                create create { primary; accept [ledger_id, memo]; }
            }
        }
    }
}

mod scoped {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Doc {
            table "subrpc_docs";

            multitenancy {
                strategy: context;
            }

            attributes {
                id: Uuid [pk];
                title: String;
            }

            actions {
                read read { primary; }
                create create { primary; accept [title]; }
                update rename { primary; accept [title]; }
                destroy destroy { primary; }
            }
        }
    }
}

use entry::Entry;
use ledger::Ledger;
use scoped::Doc;

fn viewer() -> Actor {
    Actor::new(Uuid::new_v4()).with_attr("role", Value::from("viewer"))
}

fn rpc() -> Rpc<Memory> {
    Rpc::new()
        .action::<Ledger>("list_ledgers", "read")
        .action::<Ledger>("create_ledger", "create")
        .action::<Ledger>("rename_ledger", "rename")
        .action::<Ledger>("destroy_ledger", "destroy")
}

async fn seeded(ctx: &Context<Memory>, name: &str) -> Uuid {
    let id = Uuid::new_v4();
    Ledger::create(ctx)
        .id(id)
        .name(name)
        .secret(Some(format!("{name} secret")))
        .amount(21)
        .await
        .unwrap();
    for _ in 0..3 {
        Entry::create(ctx)
            .ledger_id(id)
            .memo(Some(format!("{name} memo")))
            .await
            .unwrap();
    }
    id
}

const FIELDS: [&str; 5] = ["name", "secret", "entryCount", "doubled", "firstMemo"];

/// Destroy, update and create answer their selected fields as the viewer's read does: the
/// attribute hidden by a field policy reads as null. Field policies can't name aggregates
/// or calculations in the DSL, so `firstMemo` (an aggregate over a related field hidden by
/// its own field policy) is compared with the read rather than asserted null.
#[tokio::test]
async fn write_results_respect_field_policies() {
    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    let rpc = rpc();
    let as_viewer = ctx_runtime.bind(Some(viewer()), ctx.tenant().map(str::to_owned));
    let target = seeded(&ctx, "doomed").await;
    let renamed = seeded(&ctx, "renamed").await;

    // Control: the viewer's read.
    let read = rpc
        .run(
            &as_viewer,
            &json!({ "action": "list_ledgers", "fields": FIELDS }),
        )
        .await;
    println!("control, viewer's read: {read}");
    for row in read["data"].as_array().unwrap() {
        assert_eq!(row["secret"], Json::Null, "control: viewer reads secret");
    }
    let read_memo = read["data"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"] == "doomed")
        .unwrap()["firstMemo"]
        .clone();

    let updated = rpc
        .run(&as_viewer, &json!({ "action": "rename_ledger", "identity": renamed, "input": { "name": "renamed2" }, "fields": FIELDS }))
        .await;
    println!("update as viewer: {updated}");
    let created = rpc
        .run(&as_viewer, &json!({ "action": "create_ledger", "input": { "id": Uuid::new_v4(), "name": "fresh", "secret": "fresh secret", "amount": 5 }, "fields": FIELDS }))
        .await;
    println!("create as viewer: {created}");
    let destroyed = rpc
        .run(
            &as_viewer,
            &json!({ "action": "destroy_ledger", "identity": target, "fields": FIELDS }),
        )
        .await;
    println!("destroy as viewer: {destroyed}");

    for (what, answer) in [
        ("update", &updated),
        ("create", &created),
        ("destroy", &destroyed),
    ] {
        assert_eq!(answer["success"], json!(true), "{what}: {answer}");
        assert_eq!(
            answer["data"]["secret"],
            Json::Null,
            "{what} returned secret past its field policy: {answer}"
        );
    }
    assert_eq!(
        destroyed["data"]["firstMemo"], read_memo,
        "destroy answered an aggregate differently from the read"
    );
}

/// Tenant sourcing: the server's context says tenant "a" (as a `rpc_router` context
/// closure would set it from an authenticated header). A client-sent `tenant` should not
/// let it read tenant "b".
#[tokio::test]
async fn client_tenant_does_not_override_the_servers_tenant() {
    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into())))
        .title("a's doc")
        .await
        .unwrap();
    Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("b").into())))
        .title("b's doc")
        .await
        .unwrap();
    let rpc = Rpc::new().action::<Doc>("list_docs", "read");
    let server_ctx = ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into()));

    let control = rpc
        .run(
            &server_ctx,
            &json!({ "action": "list_docs", "fields": ["title"] }),
        )
        .await;
    println!("control, no tenant in payload: {control}");
    assert_eq!(control["data"], json!([{ "title": "a's doc" }]));

    let switched = rpc
        .run(
            &server_ctx,
            &json!({ "action": "list_docs", "fields": ["title"], "tenant": "b" }),
        )
        .await;
    println!("payload tenant \"b\": {switched}");
    assert_eq!(
        switched["data"],
        json!([{ "title": "a's doc" }]),
        "payload tenant overrode the server's tenant"
    );
    let unbound = rpc
        .run(
            &ctx,
            &json!({ "action": "list_docs", "fields": ["title"], "tenant": "a" }),
        )
        .await;
    assert_eq!(
        unbound["success"], false,
        "payload must not supply a missing server tenant: {unbound}"
    );
    assert_eq!(unbound["errors"][0]["type"], "tenant_required");
}

/// The same over the Phoenix channel transport: a socket whose context is tenant "a".
#[cfg(feature = "axum")]
#[tokio::test]
async fn channel_payload_tenant_does_not_override_the_sockets_tenant() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into())))
        .title("a's doc")
        .await
        .unwrap();
    Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("b").into())))
        .title("b's doc")
        .await
        .unwrap();
    let rpc = Rpc::new().action::<Doc>("list_docs", "read");
    let socket_ctx = ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into()));
    let app = ash_typescript::rpc::channel::rpc_channel_router(rpc, move |_, _| socket_ctx.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let (mut socket, _) =
        tokio_tungstenite::connect_async(format!("ws://{addr}/socket/websocket?vsn=2.0.0"))
            .await
            .unwrap();
    let mut push = async |message: Json| -> Json {
        socket
            .send(Message::Text(message.to_string().into()))
            .await
            .unwrap();
        loop {
            if let Some(Ok(Message::Text(text))) = socket.next().await {
                return serde_json::from_str(&text).unwrap();
            }
        }
    };
    let topic = "ash_typescript_rpc:me";
    push(json!(["1", "1", topic, "phx_join", {}])).await;
    let control =
        push(json!(["1", "2", topic, "run", { "action": "list_docs", "fields": ["title"] }])).await;
    assert_eq!(
        control[4]["response"]["data"],
        json!([{ "title": "a's doc" }])
    );
    println!("channel control, no tenant: {}", control[4]["response"]);
    let switched = push(json!(["1", "3", topic, "run", { "action": "list_docs", "fields": ["title"], "tenant": "b" }])).await;
    println!("channel payload tenant \"b\": {}", switched[4]["response"]);
    assert_eq!(
        switched[4]["response"]["data"],
        json!([{ "title": "a's doc" }]),
        "payload tenant overrode the socket's tenant"
    );
    server.abort();
}

/// Filter and sort input on a hidden field and through a relationship: the viewer can't
/// find ledgers by values it can't read.
#[tokio::test]
async fn filter_and_sort_inputs_are_guarded() {
    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    let rpc = rpc();
    let as_viewer = ctx_runtime.bind(Some(viewer()), ctx.tenant().map(str::to_owned));
    let admin = ctx_runtime.bind(
        Some(Actor::new(Uuid::new_v4()).with_attr("role", Value::from("admin"))),
        ctx.tenant().map(str::to_owned),
    );
    seeded(&ctx, "one").await;

    let by_secret = json!({ "action": "list_ledgers", "fields": ["name"], "filter": { "secret": { "eq": "one secret" } } });
    let by_related = json!({ "action": "list_ledgers", "fields": ["name"], "filter": { "entries": { "memo": { "eq": "one memo" } } } });
    let by_sort = json!({ "action": "list_ledgers", "fields": ["name"], "sort": "secret" });
    println!(
        "sort on hidden secret as viewer: {}",
        rpc.run(&as_viewer, &by_sort).await
    );
    for (what, request) in [("secret", by_secret), ("entries.memo", by_related)] {
        let as_admin = rpc.run(&admin, &request).await;
        let viewer_answer = rpc.run(&as_viewer, &request).await;
        println!(
            "filter on {what}: admin={} viewer={}",
            as_admin["data"], viewer_answer["data"]
        );
        assert_eq!(
            as_admin["data"],
            json!([{ "name": "one" }]),
            "control: admin filter on {what}: {as_admin}"
        );
        assert_eq!(
            viewer_answer["data"],
            json!([]),
            "viewer found a ledger by hidden {what}: {viewer_answer}"
        );
    }
}

#[tokio::test]
async fn payload_tenant_cannot_rebind_writes_or_validation() {
    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    let a = ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into()));
    let b = ctx_runtime.bind(ctx.actor().cloned(), Some(("b").into()));
    let theirs = Doc::create(&b).title("b's doc").await.unwrap();
    let rpc = Rpc::new()
        .action::<Doc>("create_doc", "create")
        .action::<Doc>("rename_doc", "rename")
        .action::<Doc>("destroy_doc", "destroy");
    let request = json!({ "action": "rename_doc", "identity": theirs.id, "input": { "title": "stolen" }, "fields": ["title"], "tenant": "b" });
    assert_eq!(rpc.validate(&a, &request).await["success"], false);
    assert_eq!(rpc.run(&a, &request).await["success"], false);
    let destroyed = rpc.run(&a, &json!({ "action": "destroy_doc", "identity": theirs.id, "fields": ["title"], "tenant": "b" })).await;
    assert_eq!(destroyed["success"], true, "{destroyed}");
    assert_eq!(destroyed["data"]["title"], Json::Null);
    assert_eq!(Doc::get(&b, theirs.id).await.unwrap().title, "b's doc");
    let created = rpc.run(&a, &json!({ "action": "create_doc", "input": { "title": "a's new doc" }, "fields": ["title"], "tenant": "b" })).await;
    assert_eq!(created["success"], true, "{created}");
    assert_eq!(Doc::query(&a).all().await.unwrap().len(), 1);
    assert_eq!(Doc::query(&b).all().await.unwrap().len(), 1);
}

#[cfg(feature = "axum")]
#[tokio::test]
async fn http_payload_tenant_cannot_override_adapter_context() {
    use axum::{body::Body, http::Request};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let ctx_runtime = ash_core::Runtime::new(Memory::new());
    let ctx = ctx_runtime.bootstrap();
    Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into())))
        .title("a's doc")
        .await
        .unwrap();
    let theirs = Doc::create(&ctx_runtime.bind(ctx.actor().cloned(), Some(("b").into())))
        .title("b's doc")
        .await
        .unwrap();
    let rpc = Rpc::new()
        .action::<Doc>("list_docs", "read")
        .action::<Doc>("rename_doc", "rename");
    let app = ash_typescript::rpc::axum::rpc_router(rpc, move |headers| {
        assert_eq!(
            headers["authorization"],
            "Bearer adapter-verified-credential"
        );
        ctx_runtime.bind(ctx.actor().cloned(), Some(("a").into()))
    });
    for (path, request, success) in [
        (
            "/rpc/run",
            json!({ "action": "list_docs", "fields": ["title"], "tenant": "b" }),
            true,
        ),
        (
            "/rpc/validate",
            json!({ "action": "rename_doc", "identity": theirs.id, "input": { "title": "stolen" }, "tenant": "b" }),
            false,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer adapter-verified-credential")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let answer: Json = serde_json::from_slice(&body).unwrap();
        assert_eq!(answer["success"], success, "{answer}");
        if success {
            assert_eq!(answer["data"], json!([{ "title": "a's doc" }]));
        }
    }
}
