#[path = "../../ash-core/tests/support/guarded_upserts.rs"]
mod support;

async fn setup() -> ash_core::Runtime<ash_postgres::Postgres> {
    static INSTALLED: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    let url =
        std::env::var("DATABASE_URL").expect("DATABASE_URL is required for guarded upsert tests");
    let data = ash_postgres::Postgres::connect(&url).await.unwrap();
    INSTALLED
        .get_or_init(|| async {
            data.install(support::RESOURCES).await.unwrap();
        })
        .await;
    ash_core::Runtime::new(data)
}
support::cases!(setup);
