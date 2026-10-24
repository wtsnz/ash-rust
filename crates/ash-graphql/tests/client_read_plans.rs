#[path = "../../ash-core/tests/support/client_reads.rs"]
mod support;

use ash_core::{DataLayer, KeysetCursor, Runtime, Value};
use ash_graphql::AshGraphQL;
use async_graphql::{Request, Response};
use serde_json::{Value as Json, json};
use support::*;

fn data(response: Response) -> Json {
    assert!(
        response
            .errors
            .iter()
            .all(|error| error.message == "forbidden field"),
        "{:?}",
        response.errors
    );
    response.data.into_json().unwrap()
}

async fn read_plans<D: DataLayer + Clone + 'static>(runtime: Runtime<D>) {
    seed(&runtime).await;
    let schema = AshGraphQL::from_resources(RESOURCES).finish::<D>().unwrap();
    for (actor, expected) in [
        (admin(), json!([{"title":"private"},{"title":"public"}])),
        (member(), json!([{"title":"public"}])),
    ] {
        let ctx = reader(&runtime, actor);
        let query = r#"{ listPosts(filter: {tags: {name: {isNil: false}}}, sort: [{field: TITLE}]) {title} }"#;
        let response = data(schema.execute(Request::new(query).data(ctx.clone())).await);
        assert_eq!(response["listPosts"], expected);
        let response = data(schema.execute(Request::new(r#"{ listPosts(filter: {title: {eq: "private"}}) {title tagCount tags {name}} }"#).data(ctx)).await);
        let visible = expected.as_array().unwrap().len() == 2;
        assert_eq!(response["listPosts"][0]["tagCount"], usize::from(visible));
        assert_eq!(
            response["listPosts"][0]["tags"].as_array().unwrap().len(),
            usize::from(visible)
        );
    }
    let ctx = reader(&runtime, member());
    let first = data(
        schema
            .execute(
                Request::new("{ listPayrolls(first: 1) {results {name salary lookup} endKeyset} }")
                    .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(
        first["listPayrolls"]["results"],
        json!([{"name":"high","salary":null,"lookup":null}])
    );
    let token = first["listPayrolls"]["endKeyset"].as_str().unwrap();
    assert!(token.starts_with("v1."));
    assert!(KeysetCursor::decode(token).is_none());
    let second = data(schema.execute(Request::new(format!("{{ listPayrolls(first: 1, after: {token:?}) {{results {{name salary}} startKeyset}} }}")).data(ctx.clone())).await);
    assert_eq!(
        second["listPayrolls"]["results"],
        json!([{"name":"middle","salary":null}])
    );
    let middle = second["listPayrolls"]["startKeyset"].as_str().unwrap();
    let previous = data(
        schema
            .execute(
                Request::new(format!(
                    "{{ listPayrolls(last: 1, before: {middle:?}) {{results {{name salary}}}} }}"
                ))
                .data(ctx.clone()),
            )
            .await,
    );
    assert_eq!(
        previous["listPayrolls"]["results"],
        json!([{"name":"high","salary":null}])
    );
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
        let response = schema
            .execute(
                Request::new(format!(
                    "{{ listPayrolls(first: 1, after: {invalid:?}) {{results {{name}}}} }}"
                ))
                .data(ctx.clone()),
            )
            .await;
        assert!(
            response
                .errors
                .iter()
                .any(|error| error.message.contains("invalid or expired keyset cursor")),
            "{response:?}"
        );
    }
    let response = data(
        schema
            .execute(Request::new("{ bySalaryPayrolls(salary: 200) {name} }").data(ctx))
            .await,
    );
    assert_eq!(response["bySalaryPayrolls"], json!([]));
}

#[tokio::test]
async fn memory_client_traversals_and_sealed_hidden_sort_pages() {
    read_plans(Runtime::new(ash_memory::Memory::new())).await;
}

#[tokio::test]
async fn sqlite_client_traversals_and_sealed_hidden_sort_pages() {
    let sqlite = ash_sqlite::Sqlite::memory().await.unwrap();
    sqlite.install(RESOURCES).await.unwrap();
    read_plans(Runtime::new(sqlite)).await;
}

#[tokio::test]
async fn subscriptions_reject_relationship_and_derived_snapshot_filters_at_registration() {
    use futures_util::StreamExt;
    let runtime = Runtime::new(ash_memory::Memory::new());
    seed(&runtime).await;
    let schema = AshGraphQL::from_resources(RESOURCES)
        .with_pubsub(ash_pubsub::PubSub::new())
        .finish::<ash_memory::Memory>()
        .unwrap();
    let ctx = reader(&runtime, member());
    for query in [
        "subscription {postCreated(filter: {tags: {name: {eq: \"private\"}}}) {created {title}}}",
        "subscription {postDestroyed(filter: {tags: {name: {eq: \"private\"}}}) {destroyed}}",
        "subscription {postCreated(filter: {tagCount: {greaterThan: 0}}) {created {title}}}",
        "subscription {payrollCreated(filter: {doubled: {greaterThan: 0}}) {created {name}}}",
    ] {
        let mut stream = schema.execute_stream(Request::new(query).data(ctx.clone()));
        let response = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
            .await
            .unwrap()
            .unwrap();
        assert!(
            response.errors.iter().any(|error| error
                .message
                .contains("subscription filters cannot depend on")),
            "{query}: {response:?}"
        );
    }
}

#[tokio::test]
async fn subscriptions_reject_default_read_predicates_that_need_relationship_data() {
    use ash_core::{ActionDef, Filter, PreparationDef, Resource, ResourceDef, TrustedExpr};
    use futures_util::StreamExt;
    static READ: &[ActionDef] =
        &[ActionDef::read("read")
            .primary()
            .preparations(&[PreparationDef::Filter(|| {
                TrustedExpr::Filter(Filter::related("tags", Filter::True))
            })])];
    let resource = Box::leak(Box::new(ResourceDef {
        actions: READ,
        ..Post::DEF
    }));
    let resources: Vec<_> = RESOURCES
        .iter()
        .map(|def| if def.name == "Post" { &*resource } else { *def })
        .collect();
    let runtime = Runtime::new(ash_memory::Memory::new());
    let schema = AshGraphQL::from_resources(&resources)
        .with_pubsub(ash_pubsub::PubSub::new())
        .finish::<ash_memory::Memory>()
        .unwrap();
    let mut stream = schema.execute_stream(
        Request::new("subscription {postCreated {created {title}}}")
            .data(reader(&runtime, member())),
    );
    let response = tokio::time::timeout(std::time::Duration::from_secs(2), stream.next())
        .await
        .unwrap()
        .unwrap();
    assert!(
        response.errors.iter().any(|error| error
            .message
            .contains("subscription filters cannot depend on relationships")),
        "{response:?}"
    );
}
