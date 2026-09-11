use ash_core::{Actor, Context, Error, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod user_profile_mod {
    use super::*;

    resource! {
        UserProfile {
        table "user_profiles";

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            user_id: Uuid;
            name: String;
            ssn: Option<String>;
            salary: Option<i64>;
        }

        policies {
            policy always {
                authorize_if actor_present;
            }
        }

        field_policies {
            field ssn {
                authorize_if relates_to_actor(user_id);
                authorize_if actor_attribute_equals(role, "admin");
            }
            field salary {
                authorize_if actor_attribute_equals(role, "admin");
            }
        }

        actions {
            create create {
                primary;
                accept [user_id, name, ssn, salary];
            }

            read read {
                primary;
            }

            update update {
                primary;
                accept [name, ssn, salary];
            }
        }
    }}
}
pub use user_profile_mod::UserProfile;

pub mod department_mod {
    use super::*;

    resource! {
        Department {
        table "departments";

        attributes {
            id: Uuid [pk];
            title: String;
            manager_id: Option<Uuid>;
        }

        relationships {
            belongs_to manager: UserProfile [fk: manager_id];
        }

        policies {
            policy always {
                authorize_if actor_present;
            }
        }

        actions {
            create create {
                primary;
                accept [title, manager_id];
            }

            read read {
                primary;
            }
        }
    }}
}
pub use department_mod::Department;

#[tokio::test]
async fn test_relationship_loading_enforces_field_redaction() {
    let admin_id = Uuid::new_v4();
    let alice_id = Uuid::new_v4();
    let bob_id = Uuid::new_v4();

    let admin = Actor::new(admin_id).with_role("admin");
    let bob = Actor::new(bob_id).with_role("member");

    let mem = Memory::new();
    let admin_ctx = Context::new(mem.clone()).with_actor(admin);
    let bob_ctx = Context::new(mem.clone()).with_actor(bob);

    // 1. Admin creates Alice's profile with sensitive salary and SSN
    let alice_profile = UserProfile::create(&admin_ctx)
        .user_id(alice_id)
        .name("Alice Smith")
        .ssn(Some("123-45-6789".into()))
        .salary(Some(120_000))
        .await
        .unwrap();

    // 2. Admin creates a Department linked to Alice
    let dept = Department::create(&admin_ctx)
        .title("Engineering")
        .manager_id(Some(alice_profile.id))
        .await
        .unwrap();

    // 3. Bob reads the department and loads the related manager profile
    let dept_by_bob = Department::query(&bob_ctx)
        .filter(Department::id.eq(dept.id))
        .include(Department::manager)
        .one()
        .await
        .unwrap();

    let related_manager = dept_by_bob
        .manager
        .as_option()
        .unwrap()
        .expect("related manager must be loaded");

    // Invariant: Relationship loading MUST NOT bypass field-level policies!
    assert_eq!(related_manager.name, "Alice Smith");
    assert_eq!(
        related_manager.ssn, None,
        "SSN on related resource must be redacted for non-admin"
    );
    assert_eq!(
        related_manager.salary, None,
        "Salary on related resource must be redacted for non-admin"
    );

    // 4. Admin reads the department and loads manager: sees full data
    let dept_by_admin = Department::query(&admin_ctx)
        .filter(Department::id.eq(dept.id))
        .include(Department::manager)
        .one()
        .await
        .unwrap();

    let admin_manager = dept_by_admin
        .manager
        .as_option()
        .unwrap()
        .expect("manager loaded for admin");
    assert_eq!(admin_manager.ssn, Some("123-45-6789".into()));
    assert_eq!(admin_manager.salary, Some(120_000));
}

#[tokio::test]
async fn test_field_policies_redaction_and_authorization_in_memory() {
    let admin_id = Uuid::new_v4();
    let alice_id = Uuid::new_v4();
    let bob_id = Uuid::new_v4();

    let admin = Actor::new(admin_id).with_role("admin");
    let alice = Actor::new(alice_id).with_role("member");
    let bob = Actor::new(bob_id).with_role("member");

    let mem = Memory::new();
    let admin_ctx = Context::new(mem.clone()).with_actor(admin);

    // 1. Admin creates profile for Alice
    let alice_profile = UserProfile::create(&admin_ctx)
        .user_id(alice_id)
        .name("Alice Smith")
        .ssn(Some("123-45-6789".into()))
        .salary(Some(120_000))
        .await
        .expect("admin creates alice profile");

    assert_eq!(alice_profile.ssn, Some("123-45-6789".into()));
    assert_eq!(alice_profile.salary, Some(120_000));

    // 2. Alice reads own profile:
    // Should see ssn (relates to actor alice_id), but salary should be redacted to None (requires admin)!
    let alice_ctx = Context::new(mem.clone()).with_actor(alice.clone());
    let fetched_by_alice = UserProfile::query(&alice_ctx)
        .first()
        .await
        .expect("alice query")
        .expect("alice profile found");

    assert_eq!(fetched_by_alice.name, "Alice Smith");
    assert_eq!(fetched_by_alice.ssn, Some("123-45-6789".into()));
    assert_eq!(
        fetched_by_alice.salary, None,
        "salary must be redacted for non-admin"
    );

    // 3. Bob reads Alice's profile:
    // Neither ssn nor salary are permitted for Bob -> both redacted to None!
    let bob_ctx = Context::new(mem.clone()).with_actor(bob.clone());
    let fetched_by_bob = UserProfile::query(&bob_ctx)
        .first()
        .await
        .expect("bob query")
        .expect("alice profile found");

    assert_eq!(fetched_by_bob.name, "Alice Smith");
    assert_eq!(fetched_by_bob.ssn, None, "ssn must be redacted for bob");
    assert_eq!(
        fetched_by_bob.salary, None,
        "salary must be redacted for bob"
    );

    // 4. Alice attempts to update salary -> must be rejected by field policy on write
    let update_res = UserProfile::update(&alice_ctx, fetched_by_alice.id)
        .salary(Some(150_000))
        .await;

    assert!(
        matches!(update_res, Err(Error::Forbidden)),
        "alice cannot write to salary"
    );

    // 5. Alice updating permitted field (name) -> succeeds
    let alice_update_ok = UserProfile::update(&alice_ctx, fetched_by_alice.id)
        .name("Alice Wonder")
        .await
        .expect("alice can update permitted name");

    assert_eq!(alice_update_ok.name, "Alice Wonder");

    // 6. Admin reads Alice's profile again: full visibility
    let fetched_by_admin = UserProfile::query(&admin_ctx)
        .first()
        .await
        .expect("admin query")
        .expect("alice profile found");

    assert_eq!(fetched_by_admin.name, "Alice Wonder");
    assert_eq!(fetched_by_admin.ssn, Some("123-45-6789".into()));
    assert_eq!(fetched_by_admin.salary, Some(120_000));
}

#[tokio::test]
async fn test_field_policies_redaction_and_authorization_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&UserProfile::DEF]).await.unwrap();

    let admin_id = Uuid::new_v4();
    let user_id = Uuid::new_v4();

    let admin = Actor::new(admin_id).with_role("admin");
    let user = Actor::new(user_id).with_role("member");

    let admin_ctx = Context::new(sqlite.clone()).with_actor(admin);

    let profile = UserProfile::create(&admin_ctx)
        .user_id(user_id)
        .name("Charlie")
        .ssn(Some("987-65-4321".into()))
        .salary(Some(95_000))
        .await
        .unwrap();

    // User context in SQLite
    let user_ctx = Context::new(sqlite.clone()).with_actor(user);

    let user_view = ash_core::get::<UserProfile, _>(&user_ctx, profile.id)
        .await
        .unwrap();
    assert_eq!(user_view.ssn, Some("987-65-4321".into()));
    assert_eq!(user_view.salary, None, "salary redacted in sqlite");

    // Attempting unauthorized write in SQLite
    let forbidden = UserProfile::update(&user_ctx, profile.id)
        .salary(Some(200_000))
        .await;
    assert!(matches!(forbidden, Err(Error::Forbidden)));

    // Permitted write in SQLite preserves redacted field
    let updated = UserProfile::update(&user_ctx, profile.id)
        .name("Charlie Brown")
        .await
        .unwrap();
    assert_eq!(updated.name, "Charlie Brown");

    // Admin verifies salary was preserved in SQLite
    let admin_view = ash_core::get::<UserProfile, _>(&admin_ctx, profile.id)
        .await
        .unwrap();
    assert_eq!(admin_view.name, "Charlie Brown");
    assert_eq!(
        admin_view.salary,
        Some(95_000),
        "salary preserved in sqlite"
    );
}
