#[path = "support/trusted_nodes.rs"]
mod support;
use ash_core::{ClientExpr, DataLayerKind, Error, FieldMap, Filter, ReadLock, Resource, Runtime};
use support::*;

#[tokio::test]
async fn sqlite_fragments_filter_sort_and_calculate_with_tenancy_and_policies() {
    let data = ash_sqlite::Sqlite::memory().await.unwrap();
    data.install(RESOURCES).await.unwrap();
    fragment_results(Runtime::new(data)).await;
}

#[tokio::test]
async fn unsupported_nodes_fail_before_reading_missing_tables() {
    let memory = Runtime::new(ash_memory::Memory::new());
    let error = Document::query(&reader(&memory))
        .action("matching")
        .argument("term", "rust")
        .all()
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::UnsupportedPlanNode {
            node: "sql_fragment",
            data_layer: DataLayerKind::Memory
        }
    ));
    let memory_ctx = reader(&memory);
    for query in [
        Document::query(&memory_ctx).calc(Document::public_length),
        Document::query(&memory_ctx).action("ranked"),
    ] {
        assert!(matches!(
            query.all().await,
            Err(Error::UnsupportedPlanNode {
                node: "sql_fragment",
                ..
            })
        ));
    }
    let sqlite = Runtime::new(ash_sqlite::Sqlite::memory().await.unwrap());
    let ctx = reader(&sqlite);
    let error = Document::query(&ctx)
        .action("search")
        .argument("term", "rust")
        .all()
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::UnsupportedPlanNode {
            node: "sql_fragment",
            data_layer: DataLayerKind::Sqlite
        }
    ));
    let error = Document::query(&ctx)
        .distinct_on(["category"])
        .all()
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        Error::UnsupportedPlanNode {
            node: "distinct_on",
            ..
        }
    ));
    assert!(matches!(
        Document::query(&ctx).for_update().all().await,
        Err(Error::LockRequiresTransaction)
    ));
    ctx.transaction(|ctx| async move {
        assert!(matches!(
            Document::query(&ctx).lock(ReadLock::ForUpdate).all().await,
            Err(Error::UnsupportedPlanNode {
                node: "row_lock",
                ..
            })
        ));
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn client_expressions_and_json_cannot_supply_sql() {
    let runtime = Runtime::new(ash_memory::Memory::new());
    let ctx = reader(&runtime);
    let forged = ClientExpr::new(
        TITLE_MATCH
            .bind(ash_core::fields! {"term" => "rust"})
            .into_filter(),
    )
    .into_filter();
    assert!(matches!(
        ash_core::plan_filter(
            &Document::DEF,
            ctx.actor(),
            ctx.tenant(),
            ash_core::TrustedExpr::Filter(forged)
        ),
        Err(Error::Invalid(_))
    ));
    assert!(
        ash_core::input::filter_input(&Document::DEF, &serde_json::json!({"fragment":"1=1"}))
            .is_err()
    );
    assert_eq!(
        Filter::DataLayer(match TITLE_MATCH.bind(FieldMap::new()).into_filter() {
            Filter::DataLayer(node) => node,
            _ => unreachable!(),
        })
        .snapshot_match(&Document::DEF, &FieldMap::new()),
        ash_core::FilterMatch::NeedsData
    );
}
