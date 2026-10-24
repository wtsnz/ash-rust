#[path = "../../ash-core/tests/support/derived_values.rs"]
mod support;

use ash_core::{DataLayer, Runtime, SchemaSupport};
use ash_graphql::AshGraphQL;
use async_graphql::{Request, Response};
use serde_json::{Value as Json, json};
use support::*;

fn data(response: Response) -> Json {
    assert!(
        response
            .errors
            .iter()
            .all(|e| e.message == "forbidden field"),
        "{:?}",
        response.errors
    );
    response.data.into_json().unwrap()
}

async fn visibility<D: DataLayer + Clone + 'static>(runtime: Runtime<D>) {
    let id = seed(&runtime).await;
    let schema = AshGraphQL::from_resources(RESOURCES).finish::<D>().unwrap();
    let ctx = runtime.bind(Some(member()), None);
    let response = schema.execute(Request::new(r#"{ listEmployees(filter: {name: {eq: "hidden"}}) {
        name salaryX2 ssnLength fallback conditional restricted ssnLast4 privateLast4 restrictedLast4 guardedCustom profileSecret profileSuffix
    }}"#).data(ctx.clone())).await;
    assert!(
        response
            .errors
            .iter()
            .any(|e| e.message == "forbidden field")
    );
    assert_eq!(
        data(response)["listEmployees"],
        json!([{"name":"hidden", "salaryX2":null,"ssnLength":null,
        "fallback":null,"conditional":null,"restricted":null,"ssnLast4":"0001","privateLast4":null,"restrictedLast4":null,"guardedCustom":null,"profileSecret":null,"profileSuffix":"vate"}])
    );
    let sorted = data(schema.execute(Request::new("{ listEmployees(sort: [{field: SSN_LENGTH, order: DESC}, {field: NAME}]) {name ssnLength} }").data(ctx.clone())).await);
    assert_eq!(
        sorted["listEmployees"],
        json!([{"name":"hidden","ssnLength":null},{"name":"large","ssnLength":null},{"name":"small","ssnLength":null}])
    );
    let sorted = data(
        schema
            .execute(
                Request::new(
                    "{ listEmployees(sort: [{field: SALARY_X2, order: DESC_NULLS_LAST}]) {name} }",
                )
                .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(
        sorted["listEmployees"],
        json!([{"name":"large"},{"name":"small"},{"name":"hidden"}])
    );
    for query in [
        r#"{ listEmployees(filter: {salaryX2: {greaterThan: 500}}) {name} }"#,
        r#"{ listEmployees(filter: {fallback: {eq: "111-11-0001"}}) {name} }"#,
        r#"{ listEmployees(filter: {restricted: {eq: "hidden"}}) {name} }"#,
        r#"{ listDepartments(filter: {topSsn: {stringStartsWith: "111"}}) {title} }"#,
        r#"{ listDepartments(filter: {totalSalary: {greaterThan: 500}}) {title} }"#,
        r#"{ listDepartments(filter: {restrictedCount: {greaterThan: 0}}) {title} }"#,
    ] {
        let result = data(schema.execute(Request::new(query).data(ctx.clone())).await);
        assert_eq!(
            result.as_object().unwrap().values().next().unwrap(),
            &json!([]),
            "{query}"
        );
    }
    let visible = data(
        schema
            .execute(
                Request::new(
                    "{ listEmployees(filter: {publicSsnLength: {eq: 11}}) {name publicSsnLength} }",
                )
                .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(visible["listEmployees"].as_array().unwrap().len(), 3);
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
        let result = data(schema.execute(Request::new("{ listDepartments {totalSalary topSsn firstSalary restrictedCount linkedSalary linkedSsn employeeCount hasEmployees} }").data(scope)).await);
        let row = &result["listDepartments"][0];
        assert_eq!(row["totalSalary"], total);
        assert_eq!(row["linkedSalary"], total);
        assert_eq!(row["topSsn"], ssn);
        assert_eq!(row["linkedSsn"], ssn);
        assert_eq!(row["restrictedCount"], count);
        assert_eq!(row["hasEmployees"], true);
    }
    let first = data(
        schema
            .execute(
                Request::new("{ rankedEmployees(first: 1) {results {name salaryX2} endKeyset} }")
                    .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(
        first["rankedEmployees"]["results"],
        json!([{"name":"hidden","salaryX2":null}])
    );
    let token = first["rankedEmployees"]["endKeyset"].as_str().unwrap();
    let second = data(schema.execute(Request::new(format!("{{ rankedEmployees(first: 1, after: {token:?}) {{results {{name salaryX2}} startKeyset}} }}")).data(ctx.clone())).await);
    assert_eq!(
        second["rankedEmployees"]["results"],
        json!([{"name":"large","salaryX2":400}])
    );
    let previous = data(
        schema
            .execute(
                Request::new(format!(
                    "{{ rankedEmployees(last: 1, before: {:?}) {{results {{name salaryX2}}}} }}",
                    second["rankedEmployees"]["startKeyset"].as_str().unwrap()
                ))
                .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(
        previous["rankedEmployees"]["results"],
        json!([{"name":"hidden","salaryX2":null}])
    );
    let write = format!(
        r#"mutation {{ updateDepartment(id: "{id}", input: {{title: "renamed"}}) {{errors {{message}} result {{totalSalary topSsn restrictedCount linkedSalary linkedSsn}}}} }}"#
    );
    let result = data(schema.execute(Request::new(write).data(ctx.clone())).await);
    assert_eq!(result["updateDepartment"]["errors"], json!([]));
    assert_eq!(
        result["updateDepartment"]["result"],
        json!({"totalSalary":300,"topSsn":null,"restrictedCount":null,"linkedSalary":300,"linkedSsn":null})
    );
    let write = r#"mutation {updateEmployee(id: "00000000-0000-0000-0000-000000000001", input: {name: "hidden"}) {result {salaryX2 fallback ssnLast4 restrictedLast4 guardedCustom profileSecret profileSuffix}}}"#;
    let result = data(schema.execute(Request::new(write).data(ctx.clone())).await);
    assert_eq!(
        result["updateEmployee"]["result"],
        json!({"salaryX2":null,"fallback":null,"ssnLast4":"0001","restrictedLast4":null,"guardedCustom":null,"profileSecret":null,"profileSuffix":"vate"})
    );
}

#[tokio::test]
async fn memory_graphql_projects_derived_reads_comparisons_and_write_loads() {
    visibility(Runtime::new(ash_memory::Memory::new())).await;
}
#[tokio::test]
async fn sqlite_graphql_projects_derived_reads_comparisons_and_write_loads() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install_resources(RESOURCES).await.unwrap();
    visibility(Runtime::new(data)).await;
}
