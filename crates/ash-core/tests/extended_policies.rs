use ash_core::{Actor, Context, Error, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    resource Document;
    table "documents";

    attributes {
        id: Uuid [pk],
        title: String,
        author_id: Uuid,
        classified: bool [default: false],
        confidential_notes: Option<String>,
    }

    actions {
        create create {
            primary true;
            accept [title, author_id, classified, confidential_notes];
        }

        read read {
            primary true;
        }

        update update {
            primary true;
            accept [title, classified, confidential_notes];
        }
    }

    policies {
        // 1. Bypass block: Admins bypass all subsequent restrictions completely
        bypass {
            authorize_if actor_attribute_equals(role, "admin");
        }

        // 2. Hard forbid: Banned actors can NEVER read or write anything, even if they are the author
        policy {
            forbid_if actor_attribute_equals(status, "banned");
            authorize_if relates_to_actor(author_id);
        }

        // 3. Negative check: Only unclassified documents can be accessed unless actor has security clearance
        policy action_type(read) {
            forbid_unless relates_to_actor(author_id);
            authorize_unless eq(classified, true);
        }
    }

    field_policies {
        // 4. Field level policy: confidential_notes is hidden if the actor is not security officer
        field confidential_notes {
            forbid_unless actor_attribute_equals(clearance, "top_secret");
            authorize_if always;
        }
    }
}

#[tokio::test]
async fn test_bypass_policy_in_memory() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    let author_id = Uuid::new_v4();
    let other_user_id = Uuid::new_v4();

    let author_actor = Actor::new(author_id)
        .with_attr("status", "active")
        .with_attr("clearance", "top_secret");
    let author_ctx = ctx.with_actor(author_actor);

    // Create a document by author
    let doc = Document::create(&author_ctx)
        .title("Secret Project")
        .author_id(author_id)
        .classified(false)
        .confidential_notes("Launch on Tuesday")
        .await
        .unwrap();

    // Normal user (not author) cannot read
    let normal_actor = Actor::new(other_user_id).with_attr("role", "user");
    let normal_ctx = ctx.with_actor(normal_actor);

    let docs = Document::query(&normal_ctx).all().await.unwrap();
    assert_eq!(docs.len(), 0, "Normal user cannot see author's doc");

    // Admin actor bypasses all checks!
    let admin_actor = Actor::new(Uuid::new_v4()).with_attr("role", "admin");
    let admin_ctx = ctx.with_actor(admin_actor);

    let admin_docs = Document::query(&admin_ctx).all().await.unwrap();
    assert_eq!(admin_docs.len(), 1, "Admin bypasses check and sees all documents");
    assert_eq!(admin_docs[0].id, doc.id);

    // Admin can also update, bypassing author ownership check
    let updated = Document::update(&admin_ctx, doc.id)
        .title("Secret Project Revised by Admin")
        .await
        .unwrap();
    assert_eq!(updated.title, "Secret Project Revised by Admin");
}

#[tokio::test]
async fn test_forbid_if_blocks_even_author() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    let author_id = Uuid::new_v4();
    let author_ctx = ctx.with_actor(Actor::new(author_id).with_attr("status", "active"));

    let doc = Document::create(&author_ctx)
        .title("My Draft")
        .author_id(author_id)
        .classified(false)
        .await
        .unwrap();

    // Author without ban can read:
    let active_author = Actor::new(author_id).with_attr("status", "active");
    let active_ctx = ctx.with_actor(active_author);
    let my_docs = Document::query(&active_ctx).all().await.unwrap();
    assert_eq!(my_docs.len(), 1);

    // Banned author is hard-forbidden by `forbid_if`
    let banned_author = Actor::new(author_id).with_attr("status", "banned");
    let banned_ctx = ctx.with_actor(banned_author);

    let banned_docs = Document::query(&banned_ctx).all().await.unwrap();
    assert_eq!(banned_docs.len(), 0, "Banned author is forbidden from querying");

    let write_res = Document::update(&banned_ctx, doc.id)
        .title("Hacked Title")
        .await;
    assert!(
        matches!(write_res, Err(Error::NotFound | Error::Forbidden)),
        "Banned user cannot update"
    );
}

#[tokio::test]
async fn test_field_policy_forbid_unless() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    let author_id = Uuid::new_v4();
    let author_create_ctx = ctx.with_actor(
        Actor::new(author_id)
            .with_attr("status", "active")
            .with_attr("clearance", "top_secret"),
    );

    let doc = Document::create(&author_create_ctx)
        .title("Architecture Spec")
        .author_id(author_id)
        .classified(false)
        .confidential_notes("Private Server Keys")
        .await
        .unwrap();

    // Author without top_secret clearance sees confidential_notes as None (redacted)
    let author_ctx = ctx.with_actor(
        Actor::new(author_id)
            .with_attr("status", "active")
            .with_attr("clearance", "standard"),
    );
    let read_doc = Document::get(&author_ctx, doc.id).await.unwrap();
    assert_eq!(read_doc.confidential_notes, None, "confidential_notes should be redacted");

    // Officer with top_secret clearance sees confidential_notes intact
    let officer_ctx = ctx.with_actor(
        Actor::new(author_id)
            .with_attr("status", "active")
            .with_attr("clearance", "top_secret"),
    );
    let officer_doc = Document::get(&officer_ctx, doc.id).await.unwrap();
    assert_eq!(
        officer_doc.confidential_notes,
        Some("Private Server Keys".into()),
        "Officer with top_secret sees unredacted notes"
    );
}

#[tokio::test]
async fn test_extended_policies_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Document::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    let author_id = Uuid::new_v4();
    let author_ctx = ctx.with_actor(Actor::new(author_id).with_attr("status", "active"));

    let doc = Document::create(&author_ctx)
        .title("SQLite Spec")
        .author_id(author_id)
        .classified(false)
        .await
        .unwrap();

    // Non-admin query
    let user_ctx = ctx.with_actor(Actor::new(Uuid::new_v4()).with_attr("role", "user"));
    let user_docs = Document::query(&user_ctx).all().await.unwrap();
    assert_eq!(user_docs.len(), 0);

    // Admin query bypass in SQL
    let admin_ctx = ctx.with_actor(Actor::new(Uuid::new_v4()).with_attr("role", "admin"));
    let admin_docs = Document::query(&admin_ctx).all().await.unwrap();
    assert_eq!(admin_docs.len(), 1);
    assert_eq!(admin_docs[0].id, doc.id);
}
