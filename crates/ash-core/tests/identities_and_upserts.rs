use ash_core::{Context, Error, Resource, SchemaSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Account {
        table "accounts";

    attributes {
        id: Uuid [pk];
        email: String;
        org_id: Uuid;
        slug: String;
        display_name: String;
        points: i64;
    }

    identities {
        identity unique_email: [email], message: "Email is already registered";
        identity org_slug: [org_id, slug];
    }

    actions {
        create register {
            primary;
            accept [email, org_id, slug, display_name, points];
        }

        read read {
            primary;
        }

        update update_profile {
            primary;
            accept [display_name, points];
        }
    }
    }}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_identities_conflict_detection_in_memory() {
    let ctx = Context::new(Memory::new());
    let org1 = Uuid::new_v4();

    // 1. Create first account
    let acc1 = Account::register(&ctx)
        .email("alice@example.com")
        .org_id(org1)
        .slug("alice")
        .display_name("Alice A")
        .points(100)
        .await
        .expect("create alice");
    assert_eq!(acc1.display_name, "Alice A");

    // 2. Conflict on unique_email
    let err_email = Account::register(&ctx)
        .email("alice@example.com") // duplicate email!
        .org_id(org1)
        .slug("alice2")
        .display_name("Alice B")
        .points(50)
        .await
        .expect_err("should reject duplicate email");

    match err_email {
        Error::IdentityConflict { identity, fields, message } => {
            assert_eq!(identity, "unique_email");
            assert_eq!(fields, vec!["email"]);
            assert_eq!(message, "Email is already registered");
        }
        other => panic!("expected IdentityConflict, got: {other:?}"),
    }

    // 3. Conflict on composite identity org_slug
    let err_slug = Account::register(&ctx)
        .email("another@example.com")
        .org_id(org1)
        .slug("alice") // duplicate slug in same org!
        .display_name("Another")
        .points(10)
        .await
        .expect_err("should reject duplicate org_slug");

    match err_slug {
        Error::IdentityConflict { identity, fields, .. } => {
            assert_eq!(identity, "org_slug");
            assert_eq!(fields, vec!["org_id", "slug"]);
        }
        other => panic!("expected IdentityConflict on org_slug, got: {other:?}"),
    }

    // 4. Same slug in a different org is valid!
    let org2 = Uuid::new_v4();
    let acc2 = Account::register(&ctx)
        .email("another@example.com")
        .org_id(org2)
        .slug("alice") // same slug, different org!
        .display_name("Alice in Org2")
        .points(10)
        .await
        .expect("same slug in different org should succeed");
    assert_eq!(acc2.slug, "alice");
    assert_eq!(acc2.org_id, org2);
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_identities_get_by_and_find_by_helpers() {
    let ctx = Context::new(Memory::new());
    let org_id = Uuid::new_v4();

    Account::register(&ctx)
        .email("bob@example.com")
        .org_id(org_id)
        .slug("bob-slug")
        .display_name("Bob")
        .points(250)
        .await
        .expect("create bob");

    // Test single-key identity helpers
    let bob_found = Account::get_by_unique_email(&ctx, "bob@example.com").await.expect("get_by email");
    assert_eq!(bob_found.display_name, "Bob");

    let bob_opt = Account::find_by_unique_email(&ctx, "bob@example.com").await.expect("find_by email");
    assert!(bob_opt.is_some());
    assert_eq!(bob_opt.unwrap().email, "bob@example.com");

    let missing_bob = Account::find_by_unique_email(&ctx, "unknown@example.com").await.expect("find_by missing");
    assert!(missing_bob.is_none());

    let missing_err = Account::get_by_unique_email(&ctx, "unknown@example.com").await;
    assert!(matches!(missing_err, Err(Error::NotFound)));

    // Test composite-key identity helpers
    let bob_composite = Account::get_by_org_slug(&ctx, org_id, "bob-slug").await.expect("get_by org_slug");
    assert_eq!(bob_composite.email, "bob@example.com");

    let bob_comp_opt = Account::find_by_org_slug(&ctx, org_id, "bob-slug").await.expect("find_by org_slug");
    assert!(bob_comp_opt.is_some());

    let wrong_org = Account::find_by_org_slug(&ctx, Uuid::new_v4(), "bob-slug").await.expect("find_by wrong org");
    assert!(wrong_org.is_none());
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_upsert_in_memory() {
    let ctx = Context::new(Memory::new());
    let org = Uuid::new_v4();

    // 1. Initial upsert (row does not exist yet -> creates row)
    let acc1 = Account::register(&ctx)
        .email("carol@example.com")
        .org_id(org)
        .slug("carol")
        .display_name("Carol Initial")
        .points(50)
        .upsert_on(Account::unique_email, &["display_name", "points"])
        .await
        .expect("initial upsert should insert");
    assert_eq!(acc1.display_name, "Carol Initial");
    assert_eq!(acc1.points, 50);

    // 2. Second upsert with same email (row exists -> updates display_name and points)
    let acc2 = Account::register(&ctx)
        .email("carol@example.com")
        .org_id(org)
        .slug("carol-changed-slug") // not listed in update_fields, should preserve original!
        .display_name("Carol Updated")
        .points(999)
        .upsert_on(Account::unique_email, &["display_name", "points"])
        .await
        .expect("upsert on existing email should update");

    assert_eq!(acc2.id, acc1.id);
    assert_eq!(acc2.display_name, "Carol Updated");
    assert_eq!(acc2.points, 999);
    assert_eq!(acc2.slug, "carol"); // preserved original slug!

    // Verify row in database
    let fetched = Account::get_by_unique_email(&ctx, "carol@example.com").await.expect("fetch carol");
    assert_eq!(fetched.display_name, "Carol Updated");
    assert_eq!(fetched.points, 999);

    // 3. Composite identity upsert on org_slug
    let comp_upsert1 = Account::register(&ctx)
        .email("emp1@org.com")
        .org_id(org)
        .slug("lead-dev")
        .display_name("Lead Dev 1")
        .points(10)
        .upsert_on(Account::org_slug, &["display_name", "points"])
        .await
        .unwrap();

    let comp_upsert2 = Account::register(&ctx)
        .email("emp2@org.com") // different email
        .org_id(org)           // same org
        .slug("lead-dev")      // same slug -> conflicts on org_slug!
        .display_name("Lead Dev 2")
        .points(99)
        .upsert_on(Account::org_slug, &["display_name", "points"])
        .await
        .unwrap();

    assert_eq!(comp_upsert2.id, comp_upsert1.id);
    assert_eq!(comp_upsert2.display_name, "Lead Dev 2");
    assert_eq!(comp_upsert2.points, 99);
    assert_eq!(comp_upsert2.email, "emp1@org.com"); // preserved original email
}

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_identities_and_upsert_in_sqlite() {
    let db = Sqlite::memory().await.expect("sqlite memory");
    db.install_resources(&[&Account::DEF]).await.expect("install schema");
    let ctx = Context::new(db);
    let org = Uuid::new_v4();

    // 1. Create account in sqlite
    Account::register(&ctx)
        .email("dave@example.com")
        .org_id(org)
        .slug("dave")
        .display_name("Dave Original")
        .points(100)
        .await
        .expect("create dave");

    // 2. Verify unique constraint conflict mapping in sqlite
    let conflict_err = Account::register(&ctx)
        .email("dave@example.com") // duplicate!
        .org_id(org)
        .slug("dave2")
        .display_name("Dave 2")
        .points(20)
        .await
        .expect_err("should reject duplicate email in sqlite");

    match conflict_err {
        Error::IdentityConflict { identity, fields, message } => {
            assert_eq!(identity, "unique_email");
            assert_eq!(fields, vec!["email"]);
            assert_eq!(message, "Email is already registered");
        }
        other => panic!("expected IdentityConflict, got: {other:?}"),
    }

    // 3. Test upsert in sqlite using generated Account::unique_email constant and upsert_on
    let upserted = Account::register(&ctx)
        .email("dave@example.com")
        .org_id(org)
        .slug("dave")
        .display_name("Dave Upserted")
        .points(777)
        .upsert_on(Account::unique_email, &["display_name", "points"])
        .await
        .expect("sqlite upsert on conflict");

    assert_eq!(upserted.display_name, "Dave Upserted");
    assert_eq!(upserted.points, 777);

    // 4. Verify composite identity upsert in SQLite
    let comp1 = Account::register(&ctx)
        .email("sqlite_emp1@org.com")
        .org_id(org)
        .slug("sqlite-lead")
        .display_name("SQL Lead 1")
        .points(50)
        .upsert_on(Account::org_slug, &["display_name", "points"])
        .await
        .unwrap();

    let comp2 = Account::register(&ctx)
        .email("sqlite_emp2@org.com")
        .org_id(org)
        .slug("sqlite-lead")
        .display_name("SQL Lead 2")
        .points(888)
        .upsert_on(Account::org_slug, &["display_name", "points"])
        .await
        .unwrap();

    assert_eq!(comp2.id, comp1.id);
    assert_eq!(comp2.display_name, "SQL Lead 2");
    assert_eq!(comp2.points, 888);

    // 5. Verify get_by helper against SQLite
    let re_read = Account::get_by_unique_email(&ctx, "dave@example.com").await.expect("get_by dave in sqlite");
    assert_eq!(re_read.display_name, "Dave Upserted");
    assert_eq!(re_read.points, 777);
}
