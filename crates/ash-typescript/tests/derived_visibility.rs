#[path = "../../ash-core/tests/support/derived_values.rs"]
mod support;

use ash_core::{DataLayer, Runtime, SchemaSupport, TransactionSupport};
use ash_typescript::rpc::Rpc;
use serde_json::{Value as Json, json};
use support::*;

fn data(response: Json) -> Json {
    assert_eq!(response["success"], true, "{response}");
    response["data"].clone()
}

async fn visibility<D: DataLayer + TransactionSupport + Clone + 'static>(runtime: Runtime<D>) {
    let id = seed(&runtime).await;
    let rpc = Rpc::new()
        .action::<Employee>("employees", "read")
        .action::<Employee>("ranked", "ranked")
        .action::<Department>("departments", "read")
        .action::<Department>("update_department", "update")
        .action::<Employee>("update_employee", "update");
    let ctx = runtime.bind(Some(member()), None);
    let read = json!({"action":"employees","filter":{"name":{"eq":"hidden"}},"fields":["name","salaryX2","ssnLength","fallback","conditional","restricted","ssnLast4","privateLast4","restrictedLast4","guardedCustom","profileSecret","profileSuffix"]});
    assert_eq!(
        data(rpc.run(&ctx, &read).await),
        json!([{"name":"hidden","salaryX2":null,"ssnLength":null,
        "fallback":null,"conditional":null,"restricted":null,"ssnLast4":"0001","privateLast4":null,"restrictedLast4":null,"guardedCustom":null,"profileSecret":null,"profileSuffix":"vate"}])
    );
    let sorted = data(
        rpc.run(
            &ctx,
            &json!({"action":"employees","sort":"-ssnLength,name","fields":["name","ssnLength"]}),
        )
        .await,
    );
    assert_eq!(
        sorted,
        json!([{"name":"hidden","ssnLength":null},{"name":"large","ssnLength":null},{"name":"small","ssnLength":null}])
    );
    let sorted = data(
        rpc.run(
            &ctx,
            &json!({"action":"employees","sort":"--salaryX2","fields":["name"]}),
        )
        .await,
    );
    assert_eq!(
        sorted,
        json!([{"name":"large"},{"name":"small"},{"name":"hidden"}])
    );
    for (action, filter) in [
        ("employees", json!({"salaryX2":{"greaterThan":500}})),
        ("employees", json!({"fallback":{"eq":"111-11-0001"}})),
        ("employees", json!({"restricted":{"eq":"hidden"}})),
        ("departments", json!({"topSsn":{"stringStartsWith":"111"}})),
        ("departments", json!({"totalSalary":{"greaterThan":500}})),
        ("departments", json!({"restrictedCount":{"greaterThan":0}})),
    ] {
        assert_eq!(
            data(
                rpc.run(
                    &ctx,
                    &json!({"action":action,"filter":filter,"fields":["id"]})
                )
                .await
            ),
            json!([])
        );
    }
    let fields = json!([
        "totalSalary",
        "topSsn",
        "restrictedCount",
        "linkedSalary",
        "linkedSsn",
        "employeeCount",
        "hasEmployees"
    ]);
    for (scope, total, ssn, count) in [
        (ctx.clone(), 300, Json::Null, Json::Null),
        (
            runtime.bind(Some(admin()), None),
            800,
            json!("111-11-0001"),
            json!(3),
        ),
        (
            runtime.system().bootstrap(),
            1800,
            json!("111-11-0001"),
            json!(4),
        ),
    ] {
        let rows = data(
            rpc.run(&scope, &json!({"action":"departments","fields":fields}))
                .await,
        );
        assert_eq!(rows[0]["totalSalary"], total);
        assert_eq!(rows[0]["linkedSalary"], total);
        assert_eq!(rows[0]["topSsn"], ssn);
        assert_eq!(rows[0]["linkedSsn"], ssn);
        assert_eq!(rows[0]["restrictedCount"], count);
        assert_eq!(rows[0]["hasEmployees"], true);
    }
    let written = data(rpc.run(&ctx, &json!({"action":"update_department","identity":id,"input":{"title":"renamed"},"fields":fields})).await);
    assert_eq!(written["totalSalary"], 300);
    assert_eq!(written["topSsn"], Json::Null);
    assert_eq!(written["restrictedCount"], Json::Null);
    assert_eq!(written["linkedSalary"], 300);
    let written = data(rpc.run(&ctx, &json!({"action":"update_employee","identity":"00000000-0000-0000-0000-000000000001","input":{"name":"hidden"},"fields":["salaryX2","fallback","ssnLast4","restrictedLast4","guardedCustom","profileSecret","profileSuffix"]})).await);
    assert_eq!(
        written,
        json!({"salaryX2":null,"fallback":null,"ssnLast4":"0001","restrictedLast4":null,"guardedCustom":null,"profileSecret":null,"profileSuffix":"vate"})
    );
    let first = data(
        rpc.run(
            &ctx,
            &json!({"action":"ranked","fields":["name","salaryX2"],"page":{"limit":1}}),
        )
        .await,
    );
    assert_eq!(first["results"], json!([{"name":"hidden","salaryX2":null}]));
    let second = data(rpc.run(&ctx, &json!({"action":"ranked","fields":["name","salaryX2"],"page":{"limit":1,"after":first["nextPage"]}})).await);
    assert_eq!(second["results"], json!([{"name":"large","salaryX2":400}]));
}

#[tokio::test]
async fn memory_rpc_projects_derived_reads_comparisons_and_write_loads() {
    visibility(Runtime::new(ash_memory::Memory::new())).await;
}
#[tokio::test]
async fn sqlite_rpc_projects_derived_reads_comparisons_and_write_loads() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install_resources(RESOURCES).await.unwrap();
    visibility(Runtime::new(data)).await;
}
