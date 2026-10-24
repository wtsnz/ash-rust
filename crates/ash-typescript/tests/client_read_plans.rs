#[path = "../../ash-core/tests/support/client_reads.rs"]
mod support;

use ash_core::{DataLayer, KeysetCursor, Runtime, Value};
use ash_typescript::rpc::{Identity, Rpc};
use serde_json::{Value as Json, json};
use support::*;

fn data(response: Json) -> Json {
    assert_eq!(response["success"], true, "{response}");
    response["data"].clone()
}

async fn read_plans<D: DataLayer + ash_core::TransactionSupport + Clone + 'static>(
    runtime: Runtime<D>,
) {
    seed(&runtime).await;
    let rpc = Rpc::new()
        .action::<Post>("posts", "read")
        .action::<Budget>("budgets", "read")
        .action::<Payroll>("payrolls", "read")
        .get_by::<Payroll>("lookup", "read", &["lookup"])
        .action_with::<Payroll>("update", "update", |options| {
            options.identities(vec![Identity::Named("by_lookup")])
        })
        .action::<Payroll>("by_salary", "by_salary");
    for (actor, expected) in [
        (admin(), json!([{"title":"private"},{"title":"public"}])),
        (member(), json!([{"title":"public"}])),
    ] {
        let response = rpc.run(&reader(&runtime, actor), &json!({"action":"posts", "fields":["title"], "filter":{"tags":{"name":{"isNil":false}}}, "sort":"title"})).await;
        assert_eq!(data(response), expected);
    }
    let ctx = reader(&runtime, member());
    let request =
        |page: Json| json!({"action":"payrolls", "fields":["name","salary","lookup"], "page":page});
    let first = data(rpc.run(&ctx, &request(json!({"limit":1}))).await);
    assert_eq!(
        first["results"],
        json!([{"name":"high","salary":null,"lookup":null}])
    );
    let token = first["nextPage"].as_str().unwrap();
    assert!(KeysetCursor::decode(token).is_none());
    let second = data(
        rpc.run(&ctx, &request(json!({"limit":1,"after":token})))
            .await,
    );
    assert_eq!(second["results"][0]["name"], "middle");
    assert!(second["results"][0]["salary"].is_null());
    let previous = data(
        rpc.run(
            &ctx,
            &request(json!({"limit":1,"before":second["previousPage"]})),
        )
        .await,
    );
    assert_eq!(previous["results"][0]["name"], "high");
    let forged = KeysetCursor {
        id: Value::Uuid(uuid::Uuid::nil()),
        values: vec![
            ("salary".into(), Value::Int(250)),
            ("id".into(), Value::Uuid(uuid::Uuid::nil())),
        ],
    }
    .encode();
    let mut tampered = token.as_bytes().to_vec();
    tampered[10] = if tampered[10] == b'A' { b'B' } else { b'A' };
    for invalid in [
        forged,
        String::from_utf8(tampered).unwrap(),
        "not-a-cursor".into(),
    ] {
        let response = rpc
            .run(&ctx, &request(json!({"limit":1,"after":invalid})))
            .await;
        assert_eq!(response["success"], false, "{response}");
        assert_eq!(response["errors"][0]["type"], "invalid_keyset");
    }
    let lookup =
        json!({"action":"lookup","getBy":{"lookup":"key-high"},"fields":["name","salary"]});
    assert_eq!(
        data(rpc.run(&reader(&runtime, admin()), &lookup).await)["name"],
        "high"
    );
    let hidden = rpc.run(&ctx, &lookup).await;
    assert_eq!(hidden["success"], false, "{hidden}");
    assert_eq!(hidden["errors"][0]["type"], "not_found");
    let update = json!({"action":"update","identity":{"lookup":"key-high"},"input":{"name":"changed"},"fields":["name"]});
    let hidden = rpc.run(&ctx, &update).await;
    assert_eq!(hidden["success"], false, "{hidden}");
    assert_eq!(hidden["errors"][0]["type"], "not_found");
    let unchanged = Payroll::query(&ctx)
        .filter(Payroll::name.eq("high"))
        .all()
        .await
        .unwrap();
    assert_eq!(unchanged.len(), 1);
    assert_eq!(
        data(rpc.run(&reader(&runtime, admin()), &update).await)["name"],
        "changed"
    );
    let args = json!({"action":"by_salary","input":{"salary":200},"fields":["name"]});
    assert_eq!(data(rpc.run(&ctx, &args).await), json!([]));
    assert_eq!(
        data(rpc.run(&reader(&runtime, admin()), &args).await)
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Nested pages retain the raw prepared sort value only inside a sealed token.
    let nested = |page: Json| json!({"action":"budgets","fields":["name",{"payrolls":{"fields":["name","salary"],"page":page}}]});
    let first = data(rpc.run(&ctx, &nested(json!({"limit":1}))).await);
    assert_eq!(first[0]["payrolls"]["results"][0]["name"], "changed");
    assert!(first[0]["payrolls"]["results"][0]["salary"].is_null());
    let token = first[0]["payrolls"]["nextPage"].as_str().unwrap();
    let second = data(
        rpc.run(&ctx, &nested(json!({"limit":1,"after":token})))
            .await,
    );
    assert_eq!(second[0]["payrolls"]["results"][0]["name"], "middle");
    let token = second[0]["payrolls"]["previousPage"].as_str().unwrap();
    let previous = data(
        rpc.run(&ctx, &nested(json!({"limit":1,"before":token})))
            .await,
    );
    assert_eq!(previous[0]["payrolls"]["results"][0]["name"], "changed");
    let next = previous[0]["payrolls"]["nextPage"].as_str().unwrap();
    assert_eq!(
        data(
            rpc.run(&ctx, &nested(json!({"limit":1,"after":next})))
                .await
        )[0]["payrolls"]["results"][0]["name"],
        "middle"
    );
}

#[tokio::test]
async fn memory_client_traversals_identities_and_sealed_hidden_sort_pages() {
    read_plans(Runtime::new(ash_memory::Memory::new())).await;
}

#[tokio::test]
async fn sqlite_client_traversals_identities_and_sealed_hidden_sort_pages() {
    let sqlite = ash_sqlite::Sqlite::memory().await.unwrap();
    sqlite.install(RESOURCES).await.unwrap();
    read_plans(Runtime::new(sqlite)).await;
}
