use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use uuid::Uuid;

use ash_core::{ChangeContext, Context, CustomChange, Error, FieldMap, Resource, Result, Value, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

// =========================================================================
// Scenario 1: Action-level shorthand and CustomChange dynamic hooks
// =========================================================================
mod test_shorthand_and_custom_change {
    use super::*;

    static BEFORE_CALLED: AtomicBool = AtomicBool::new(false);
    static AFTER_CALLED: AtomicBool = AtomicBool::new(false);
    static TX_CALLED: AtomicBool = AtomicBool::new(false);
    static AUDIT_COUNT: AtomicUsize = AtomicUsize::new(0);

    fn normalize_title(fields: &mut FieldMap) -> Result<()> {
        BEFORE_CALLED.store(true, Ordering::SeqCst);
        if let Some(Value::String(title)) = fields.get("title") {
            fields.insert("title".into(), Value::String(title.trim().to_uppercase()));
        }
        Ok(())
    }

    fn track_after(fields: &mut FieldMap) -> Result<()> {
        AFTER_CALLED.store(true, Ordering::SeqCst);
        fields.insert("tag".into(), Value::String("PROCESSED".into()));
        Ok(())
    }

    fn track_tx(res: std::result::Result<&FieldMap, &Error>) {
        if res.is_ok() {
            TX_CALLED.store(true, Ordering::SeqCst);
        }
    }

    struct DynamicAuditLogger;
    const AUDIT: DynamicAuditLogger = DynamicAuditLogger;

    impl CustomChange for DynamicAuditLogger {
        fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
            ctx.before_action(|fields| {
                fields.insert("audit_before".into(), Value::Bool(true));
                Ok(())
            });

            ctx.after_action(|_fields| {
                AUDIT_COUNT.fetch_add(1, Ordering::SeqCst);
                Ok(())
            });

            ctx.after_transaction(|res| {
                if res.is_ok() {
                    AUDIT_COUNT.fetch_add(10, Ordering::SeqCst);
                }
            });

            Ok(())
        }
    }

    resource! {
        ShorthandArticle {
        table "shorthand_articles";

        attributes {
            id: Uuid [pk];
            title: String;
            tag: Option<String>;
            audit_before: Option<bool>;
        }

        actions {
            create create {
                primary;
                accept [title];

                // Declarative shorthand action-level hooks:
                before_action normalize_title;
                after_action track_after;
                after_transaction track_tx;

                // CustomChange dynamically registering hooks:
                change custom(&AUDIT);
            }

            read read {
                primary;
            }
        }
    }}

    #[tokio::test]
    async fn test_shorthand_hooks_in_memory() {
        let ctx = Context::new(Memory::new());

        let article = ShorthandArticle::create(&ctx)
            .title("   declarative hooks   ")
            .await
            .unwrap();

        // 1. Before action transformed title:
        assert_eq!(article.title, "DECLARATIVE HOOKS");
        assert!(BEFORE_CALLED.load(Ordering::SeqCst));

        // 2. CustomChange attached before_action hook:
        assert_eq!(article.audit_before, Some(true));

        // 3. After action hook modified tag:
        assert_eq!(article.tag, Some("PROCESSED".into()));
        assert!(AFTER_CALLED.load(Ordering::SeqCst));

        // 4. After transaction hook ran:
        assert!(TX_CALLED.load(Ordering::SeqCst));

        // 5. CustomChange after_action (+1) and after_transaction (+10) ran:
        assert_eq!(AUDIT_COUNT.load(Ordering::SeqCst), 11);
    }
}

// =========================================================================
// Scenario 2: Elixir-style change-wrapped hooks on update
// =========================================================================
mod test_elixir_change_hooks {
    use super::*;

    static UPDATE_BEFORE_RAN: AtomicBool = AtomicBool::new(false);
    static UPDATE_AFTER_RAN: AtomicBool = AtomicBool::new(false);
    static UPDATE_TX_RAN: AtomicBool = AtomicBool::new(false);

    fn clean_summary(fields: &mut FieldMap) -> Result<()> {
        UPDATE_BEFORE_RAN.store(true, Ordering::SeqCst);
        if let Some(Value::String(summary)) = fields.get("summary") {
            fields.insert("summary".into(), Value::String(summary.trim().to_string()));
        }
        Ok(())
    }

    fn stamp_updated(fields: &mut FieldMap) -> Result<()> {
        UPDATE_AFTER_RAN.store(true, Ordering::SeqCst);
        fields.insert("flag".into(), Value::String("STAMPED".into()));
        Ok(())
    }

    fn tx_updated(res: std::result::Result<&FieldMap, &Error>) {
        if res.is_ok() {
            UPDATE_TX_RAN.store(true, Ordering::SeqCst);
        }
    }

    resource! {
        ElixirPost {
        table "elixir_posts";

        attributes {
            id: Uuid [pk];
            title: String;
            summary: Option<String>;
            flag: Option<String>;
        }

        actions {
            create create {
                primary;
                accept [title, summary];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [title, summary];

                // Ash Elixir change wrapped hooks:
                change before_action(clean_summary);
                change after_action(stamp_updated);
                change after_transaction(tx_updated);
            }
        }
    }}

    #[tokio::test]
    async fn test_elixir_style_hooks_on_update() {
        let ctx = Context::new(Memory::new());

        let post = ElixirPost::create(&ctx)
            .title("Post 1")
            .summary("initial")
            .await
            .unwrap();

        let updated = ElixirPost::update(&ctx, post.id)
            .summary("   spaced summary   ")
            .await
            .unwrap();

        assert_eq!(updated.summary, Some("spaced summary".into()));
        assert_eq!(updated.flag, Some("STAMPED".into()));
        assert!(UPDATE_BEFORE_RAN.load(Ordering::SeqCst));
        assert!(UPDATE_AFTER_RAN.load(Ordering::SeqCst));
        assert!(UPDATE_TX_RAN.load(Ordering::SeqCst));
    }
}

// =========================================================================
// Scenario 3: Destroy action hooks (before, after, after_transaction)
// =========================================================================
mod test_destroy_action_hooks {
    use super::*;

    static DESTROY_BEFORE_RAN: AtomicBool = AtomicBool::new(false);
    static DESTROY_AFTER_RAN: AtomicBool = AtomicBool::new(false);
    static DESTROY_TX_RAN: AtomicBool = AtomicBool::new(false);

    fn check_destroyable(fields: &mut FieldMap) -> Result<()> {
        DESTROY_BEFORE_RAN.store(true, Ordering::SeqCst);
        if let Some(Value::String(status)) = fields.get("status")
            && status == "protected"
        {
            return Err(Error::Constraint {
                field: "status".into(),
                message: "protected document cannot be destroyed".into(),
            });
        }
        Ok(())
    }

    fn record_destroyed(_fields: &mut FieldMap) -> Result<()> {
        DESTROY_AFTER_RAN.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn tx_destroyed(res: std::result::Result<&FieldMap, &Error>) {
        if res.is_ok() {
            DESTROY_TX_RAN.store(true, Ordering::SeqCst);
        }
    }

    resource! {
        Document {
        table "documents";

        attributes {
            id: Uuid [pk];
            title: String;
            status: String = "active";
        }

        actions {
            create create {
                primary;
                accept [title, status];
            }

            read read {
                primary;
            }

            destroy destroy {
                primary;
                before_action check_destroyable;
                after_action record_destroyed;
                after_transaction tx_destroyed;
            }
        }
    }}

    #[tokio::test]
    async fn test_destroy_action_lifecycle_success() {
        let ctx = Context::new(Memory::new());

        let doc = Document::create(&ctx)
            .title("Doc 1")
            .status("active")
            .await
            .unwrap();

        doc.destroy_on(&ctx).await.unwrap();

        assert!(DESTROY_BEFORE_RAN.load(Ordering::SeqCst));
        assert!(DESTROY_AFTER_RAN.load(Ordering::SeqCst));
        assert!(DESTROY_TX_RAN.load(Ordering::SeqCst));

        let docs = Document::query(&ctx).load().await.unwrap();
        assert!(docs.is_empty());
    }

    #[tokio::test]
    async fn test_destroy_action_before_hook_aborts() {
        let ctx = Context::new(Memory::new());

        let doc = Document::create(&ctx)
            .title("Doc Protected")
            .status("protected")
            .await
            .unwrap();

        let err = doc.destroy_on(&ctx).await.unwrap_err();
        match err {
            Error::Constraint { field, message } => {
                assert_eq!(field, "status");
                assert_eq!(message, "protected document cannot be destroyed");
            }
            other => panic!("expected constraint error, got: {other:?}"),
        }

        // Verify still in database
        let docs = Document::query(&ctx).load().await.unwrap();
        assert_eq!(docs.len(), 1);
    }
}

// =========================================================================
// Scenario 4: SQLite compatibility for declarative action hooks
// =========================================================================
mod test_sqlite_hooks {
    use super::*;

    fn prefix_name(fields: &mut FieldMap) -> Result<()> {
        if let Some(Value::String(name)) = fields.get("name") {
            fields.insert("name".into(), Value::String(format!("SQLITE: {name}")));
        }
        Ok(())
    }

    fn mark_saved(fields: &mut FieldMap) -> Result<()> {
        fields.insert("version".into(), Value::Int(99));
        Ok(())
    }

    resource! {
        SqliteEntity {
        table "hook_entities";

        attributes {
            id: Uuid [pk];
            name: String;
            version: Option<i64>;
        }

        actions {
            create create {
                primary;
                accept [name];

                before_action prefix_name;
                after_action mark_saved;
            }

            read read {
                primary;
            }
        }
    }}

    #[tokio::test]
    async fn test_sqlite_action_hooks() {
        let sqlite = Sqlite::memory().await.unwrap();
        let ctx = Context::new(sqlite);
        ctx.install(&[&SqliteEntity::DEF]).await.unwrap();

        let entity = SqliteEntity::create(&ctx)
            .name("widget")
            .await
            .unwrap();

        assert_eq!(entity.name, "SQLITE: widget");
        assert_eq!(entity.version, Some(99));
    }
}
