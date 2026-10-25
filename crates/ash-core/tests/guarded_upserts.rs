#[path = "support/guarded_upserts.rs"]
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

mod registry {
    use super::support;
    async fn setup() -> ash_core::Runtime<ash_core::StoreRegistry> {
        ash_core::Runtime::new(
            ash_core::StoreRegistry::new().with_default(ash_memory::Memory::new()),
        )
    }
    support::cases!(setup);
}
