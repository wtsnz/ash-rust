use ash_core::{
    resource, Context, DataLayerKind, DataLayerRegistry, Resource, SchemaSupport,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod sqlite_user {
    use super::*;

    resource! {
        User {
        table "users";
        store SqliteStore;

        attributes {
            id: Uuid [pk];
            email: String;
            name: String;
        }

        actions {
            create create {
                primary;
                accept [email, name];
            }

            read read {
                primary;
            }
        }
    }}
}

pub mod memory_session {
    use super::*;

    resource! {
        Session {
        table "sessions";
        store MemoryStore;

        attributes {
            id: Uuid [pk];
            token: String;
            user_id: Uuid;
        }

        actions {
            create create {
                primary;
                accept [token, user_id];
            }

            read read {
                primary;
            }
        }
    }}
}

pub use memory_session::Session;
pub use sqlite_user::User;

#[tokio::main(flavor = "current_thread")]
#[test]
async fn test_resource_bound_data_layer_registry() {
    // 1. Initialize SQLite (persistent storage) and Memory (ephemeral/cache storage)
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&User::DEF]).await.unwrap();
    let memory = Memory::new();

    // 2. Build multi-store registry routing by declared data layer kind
    let registry = DataLayerRegistry::new()
        .register_kind(DataLayerKind::Sqlite, sqlite.clone())
        .register_kind(DataLayerKind::Memory, memory.clone())
        .with_schema_support(sqlite);

    let ctx = Context::new(registry);

    // Invariant 1: Resource metadata correctly reflects declared data_layer
    assert_eq!(User::DEF.data_layer, DataLayerKind::Sqlite);
    assert_eq!(Session::DEF.data_layer, DataLayerKind::Memory);

    // Invariant 2: Context with registry routes User CRUD into SQLite
    let user = User::create(&ctx)
        .email("alice@example.com")
        .name("Alice")
        .call()
        .await
        .unwrap();

    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.name, "Alice");

    let loaded_users = User::query(&ctx).all().await.unwrap();
    assert_eq!(loaded_users.len(), 1);
    assert_eq!(loaded_users[0].id, user.id);

    // Invariant 3: Context with registry routes Session CRUD into Memory
    let session = Session::create(&ctx)
        .token("tok_secret_123")
        .user_id(user.id)
        .call()
        .await
        .unwrap();

    assert_eq!(session.token, "tok_secret_123");
    assert_eq!(session.user_id, user.id);

    let loaded_sessions = Session::query(&ctx).all().await.unwrap();
    assert_eq!(loaded_sessions.len(), 1);
    assert_eq!(loaded_sessions[0].id, session.id);

    // Invariant 4: Direct query to Memory reveals Session exists there, but User does not
    let mem_ctx = Context::new(memory);
    let mem_sessions = Session::query(&mem_ctx).all().await.unwrap();
    assert_eq!(mem_sessions.len(), 1);

    // Invariant 5: Multi pipeline combining steps across both data layers seamlessly
    let multi_res = ctx
        .multi()
        .create("create_bob", User::create(&ctx).email("bob@example.com").name("Bob"))
        .create_from("create_bob_session", |ctx_arg, res| {
            let bob: &User = res.get("create_bob").unwrap();
            Session::create(ctx_arg).token("tok_bob_999").user_id(bob.id).changeset()
        })
        .commit()
        .await
        .unwrap();

    let bob: &User = multi_res.get("create_bob").unwrap();
    let bob_session: &Session = multi_res.get("create_bob_session").unwrap();

    assert_eq!(bob.email, "bob@example.com");
    assert_eq!(bob_session.user_id, bob.id);

    // Verify both stores have the updated counts
    assert_eq!(User::query(&ctx).count().await.unwrap(), 2);
    assert_eq!(Session::query(&ctx).count().await.unwrap(), 2);
}
