use ash_core::{Context, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Document {
        table "documents";

    attributes {
        id: Uuid [pk];
        title: String;
        status: Option<String>;
    }

    actions {
        create create {
            primary;
            accept [title, status];
        }

        read read {
            primary;
        }
    }
    }}

#[tokio::test]
async fn test_null_inequality_identical_in_memory_and_sqlite() {
    let mem = Memory::new();
    let mem_ctx = Context::new(mem);

    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Document::DEF]).await.unwrap();
    let sql_ctx = Context::new(sqlite);

    async fn seed<D: ash_core::DataLayer>(ctx: &Context<D>) {
        Document::create(ctx)
            .title("Doc Null")
            .status(None)
            .await
            .unwrap();

        Document::create(ctx)
            .title("Doc Draft")
            .status(Some("draft".into()))
            .await
            .unwrap();

        Document::create(ctx)
            .title("Doc Published")
            .status(Some("published".into()))
            .await
            .unwrap();
    }

    seed(&mem_ctx).await;
    seed(&sql_ctx).await;

    // INVARIANT 1:
    // Filter `status != "published"` under SQL logic MUST NOT match NULL!
    // Both Memory and SQLite must return ONLY "Doc Draft"!
    let mem_ne = Document::query(&mem_ctx)
        .filter(Document::status.ne("published"))
        .all()
        .await
        .unwrap();

    let sql_ne = Document::query(&sql_ctx)
        .filter(Document::status.ne("published"))
        .all()
        .await
        .unwrap();

    assert_eq!(mem_ne.len(), 1, "Memory: NULL must not match status != 'published'");
    assert_eq!(sql_ne.len(), 1, "SQLite: NULL must not match status != 'published'");
    assert_eq!(mem_ne[0].title, "Doc Draft");
    assert_eq!(sql_ne[0].title, "Doc Draft");

    // INVARIANT 2:
    // Filter `status.is_nil() | status.ne("published")` explicitly matches both NULL and draft
    let mem_combined = Document::query(&mem_ctx)
        .filter(Document::status.is_nil() | Document::status.ne("published"))
        .all()
        .await
        .unwrap();

    let sql_combined = Document::query(&sql_ctx)
        .filter(Document::status.is_nil() | Document::status.ne("published"))
        .all()
        .await
        .unwrap();

    assert_eq!(mem_combined.len(), 2);
    assert_eq!(sql_combined.len(), 2);

    // INVARIANT 3:
    // Filter `status == "published"` matches only published in both
    let mem_eq = Document::query(&mem_ctx)
        .filter(Document::status.eq("published"))
        .all()
        .await
        .unwrap();

    let sql_eq = Document::query(&sql_ctx)
        .filter(Document::status.eq("published"))
        .all()
        .await
        .unwrap();

    assert_eq!(mem_eq.len(), 1);
    assert_eq!(sql_eq.len(), 1);
    assert_eq!(mem_eq[0].title, "Doc Published");
    assert_eq!(sql_eq[0].title, "Doc Published");

    // INVARIANT 4:
    // Filter `status.is_nil()` matches only the NULL record in both
    let mem_nil = Document::query(&mem_ctx)
        .filter(Document::status.is_nil())
        .all()
        .await
        .unwrap();

    let sql_nil = Document::query(&sql_ctx)
        .filter(Document::status.is_nil())
        .all()
        .await
        .unwrap();

    assert_eq!(mem_nil.len(), 1);
    assert_eq!(sql_nil.len(), 1);
    assert_eq!(mem_nil[0].title, "Doc Null");
    assert_eq!(sql_nil[0].title, "Doc Null");
}
