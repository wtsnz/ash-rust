#[path = "support/scoped_managed_writes.rs"]
mod support;

mod memory {
    use super::support;
    async fn setup() -> ash_core::Runtime<ash_memory::Memory> {
        ash_core::Runtime::new(ash_memory::Memory::new())
    }
    support::cases!(setup);
}

mod sqlite {
    use super::support;
    use ash_core::SchemaSupport;
    async fn setup() -> ash_core::Runtime<ash_sqlite::Sqlite> {
        let data = ash_sqlite::Sqlite::memory().await.unwrap();
        data.install_resources(support::RESOURCES).await.unwrap();
        ash_core::Runtime::new(data)
    }
    support::cases!(setup);
}

#[tokio::test]
async fn unsupported_atomicity_fails_before_writes_even_with_a_notification_buffer() {
    use ash_core::{Actor, Error, ManagedRelType, Runtime, StoreRegistry, fields};
    use support::{Article, Label, Link};
    let owner = uuid::Uuid::new_v4();
    let runtime = Runtime::new(StoreRegistry::new().with_default(ash_memory::Memory::new()));
    let ctx = runtime.bind(
        Some(Actor::new(owner).with_role("linker")),
        Some("tenant".into()),
    );
    let label = Label::create(&ctx)
        .name("label")
        .owner(owner)
        .await
        .unwrap();
    let (ctx, _) = ctx.with_notification_buffer();
    let error = Article::create(&ctx)
        .title("must not be written")
        .manage_relationship(
            "labels",
            vec![fields! { "id" => label.id }],
            ManagedRelType::Append,
        )
        .await
        .unwrap_err();
    assert!(matches!(error, Error::AtomicityUnsupported));
    assert!(Article::query(&ctx).all().await.unwrap().is_empty());
    assert!(Link::query(&ctx).all().await.unwrap().is_empty());
    let error = runtime.transaction(|_| async { Ok(()) }).await.unwrap_err();
    assert!(matches!(error, Error::AtomicityUnsupported));
    let error = ash_core::bulk_create::<Article, _, _, _>(
        &ctx,
        "create",
        [fields! { "title" => "bulk" }],
        ash_core::BulkCreateOptions::new().transaction(ash_core::BulkTransaction::All),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, Error::AtomicityUnsupported));
    assert!(Article::query(&ctx).all().await.unwrap().is_empty());
}
