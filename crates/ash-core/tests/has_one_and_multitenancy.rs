use ash_core::{
    resource, Context, Error, Filter, RelKind, Resource, SchemaSupport,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod user {
    use super::*;

    resource! {
        User {
        table "users";

        attributes {
            id: Uuid [pk];
            name: String;
        }

        relationships {
            has_one profile: super::profile::Profile [fk: user_id, on_delete: cascade];
        }

        actions {
            create create {
                primary;
                accept [name];
            }
            read read {
                primary;
            }
            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod profile {
    use super::*;

    resource! {
        Profile {
        table "profiles";

        attributes {
            id: Uuid [pk];
            user_id: Uuid;
            bio: String;
        }

        actions {
            create create {
                primary;
                accept [user_id, bio];
            }
            read read {
                primary;
            }
            destroy destroy {
                primary;
            }
        }
    }}
}

pub mod tenant_doc {
    use super::*;

    resource! {
        TenantDoc {
        table "tenant_docs";

        multitenancy {
            strategy: attribute;
            attribute: "tenant_id";
        }

        attributes {
            id: Uuid [pk];
            tenant_id: String;
            title: String;
        }

        actions {
            create create {
                primary;
                accept [title];
            }
            read read {
                primary;
            }
        }
    }}
}

pub use profile::Profile;
pub use tenant_doc::TenantDoc;
pub use user::User;

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_has_one_metadata_and_loading() {
    let user_rel = User::DEF
        .relationships
        .iter()
        .find(|r| r.name == "profile")
        .expect("profile relationship not found");
    assert_eq!(user_rel.kind, RelKind::HasOne);
    assert_eq!(user_rel.destination_attribute, "user_id");

    let mem = Memory::new();
    let ctx = Context::new(mem);

    let u = User::create(&ctx).name("Alice").call().await.unwrap();
    let p = Profile::create(&ctx)
        .user_id(u.id)
        .bio("Rust Engineer")
        .call()
        .await
        .unwrap();

    // Load user with has_one profile
    let loaded_user = User::query(&ctx)
        .filter(User::id.eq(u.id))
        .include(User::profile)
        .one()
        .await
        .unwrap();

    assert_eq!(loaded_user.name, "Alice");
    let loaded_profile = loaded_user
        .profile
        .expect_loaded("profile")
        .as_ref()
        .expect("profile should be loaded");
    assert_eq!(loaded_profile.id, p.id);
    assert_eq!(loaded_profile.bio, "Rust Engineer");
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_has_one_cascading_delete() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    let u = User::create(&ctx).name("Bob").call().await.unwrap();
    let _p = Profile::create(&ctx)
        .user_id(u.id)
        .bio("Designer")
        .call()
        .await
        .unwrap();

    // Deleting user should cascade delete the profile
    User::destroy(&ctx, u.id).await.unwrap();

    let profiles = Profile::query(&ctx).all().await.unwrap();
    assert_eq!(profiles.len(), 0, "Profile should have been cascade deleted");
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_relational_filter_has_one() {
    let mem = Memory::new();
    let ctx = Context::new(mem);

    let u1 = User::create(&ctx).name("Alice").call().await.unwrap();
    let _p1 = Profile::create(&ctx)
        .user_id(u1.id)
        .bio("Bio 1")
        .call()
        .await
        .unwrap();

    let u2 = User::create(&ctx).name("Bob").call().await.unwrap();
    let _p2 = Profile::create(&ctx)
        .user_id(u2.id)
        .bio("Bio 2")
        .call()
        .await
        .unwrap();

    // Filter Users where related profile.bio == "Bio 1"
    let users_with_bio1 = User::query(&ctx)
        .filter(Filter::related("profile", Filter::eq("bio", "Bio 1")))
        .all()
        .await
        .unwrap();

    assert_eq!(users_with_bio1.len(), 1);
    assert_eq!(users_with_bio1[0].name, "Alice");

    // Filter Users where related profile.bio == "Bio 2"
    let users_with_bio2 = User::query(&ctx)
        .filter(Filter::related("profile", Filter::eq("bio", "Bio 2")))
        .all()
        .await
        .unwrap();

    assert_eq!(users_with_bio2.len(), 1);
    assert_eq!(users_with_bio2[0].name, "Bob");
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_attribute_multitenancy() {
    let mem = Memory::new();

    // 1. Context without tenant should fail with TenantRequired on create
    let no_tenant_ctx = Context::new(mem.clone());
    let err = TenantDoc::create(&no_tenant_ctx)
        .title("Secret Doc")
        .call()
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::TenantRequired { .. }),
        "Expected TenantRequired error, got: {:?}",
        err
    );

    // 2. Context with tenant automatically populates tenant_id on create
    let tenant_a_ctx = Context::new(mem.clone()).with_tenant("org_alpha");
    let doc_a = TenantDoc::create(&tenant_a_ctx)
        .title("Alpha Plan")
        .call()
        .await
        .unwrap();
    assert_eq!(doc_a.tenant_id, "org_alpha");

    let tenant_b_ctx = Context::new(mem.clone()).with_tenant("org_beta");
    let doc_b = TenantDoc::create(&tenant_b_ctx)
        .title("Beta Plan")
        .call()
        .await
        .unwrap();
    assert_eq!(doc_b.tenant_id, "org_beta");

    // 3. Queries are scoped by tenant automatically
    let alpha_docs = TenantDoc::query(&tenant_a_ctx).all().await.unwrap();
    assert_eq!(alpha_docs.len(), 1);
    assert_eq!(alpha_docs[0].title, "Alpha Plan");

    let beta_docs = TenantDoc::query(&tenant_b_ctx).all().await.unwrap();
    assert_eq!(beta_docs.len(), 1);
    assert_eq!(beta_docs[0].title, "Beta Plan");

    // 4. Query without tenant fails with TenantRequired
    let query_err = TenantDoc::query(&no_tenant_ctx).all().await.unwrap_err();
    assert!(
        matches!(query_err, Error::TenantRequired { .. }),
        "Expected TenantRequired on query without tenant"
    );
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_has_one_and_relational_filter_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&User::DEF, &Profile::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);

    let u1 = User::create(&ctx).name("Carol").call().await.unwrap();
    let _p1 = Profile::create(&ctx)
        .user_id(u1.id)
        .bio("Lead Architect")
        .call()
        .await
        .unwrap();

    let u2 = User::create(&ctx).name("Dave").call().await.unwrap();
    let _p2 = Profile::create(&ctx)
        .user_id(u2.id)
        .bio("Product Manager")
        .call()
        .await
        .unwrap();

    // SQL EXISTS correlated subquery test: User with profile.bio = "Lead Architect"
    let carol_user = User::query(&ctx)
        .filter(Filter::related("profile", Filter::eq("bio", "Lead Architect")))
        .all()
        .await
        .unwrap();

    assert_eq!(carol_user.len(), 1);
    assert_eq!(carol_user[0].name, "Carol");
}
