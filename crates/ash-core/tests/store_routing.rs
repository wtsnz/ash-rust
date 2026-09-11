use std::any::TypeId;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ash_core::{
    CompiledQuery, Context, DataLayer, Error, FieldMap, HasStore, IdentityDef, Resource,
    ResourceDef, SchemaSupport, StoreRegistry, StoreTag, resource,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

// ============================================================================
// Test 1: Multi-instance SQLite routing (PrimaryDb vs AuditDb)
// ============================================================================

pub struct PrimaryDb;
impl StoreTag for PrimaryDb {}

pub struct AuditDb;
impl StoreTag for AuditDb {}

pub mod primary_res {
    use super::*;

    resource! {
        Customer {
        table "customers";
        store PrimaryDb;

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

pub mod audit_res {
    use super::*;

    resource! {
        AuditLog {
        table "audit_logs";
        store AuditDb;

        attributes {
            id: Uuid [pk];
            action: String;
        }

        actions {
            create create {
                primary;
                accept [action];
            }

            read read {
                primary;
            }
        }
    }}
}

pub use audit_res::AuditLog;
pub use primary_res::Customer;

#[tokio::test]
async fn test_multi_instance_sqlite_routing() {
    // 1. Two distinct SQLite instances
    let primary_sqlite = Sqlite::memory().await.unwrap();
    let audit_sqlite = Sqlite::memory().await.unwrap();

    // 2. Install schemas in their respective DBs only
    primary_sqlite.install_resources(&[&Customer::DEF]).await.unwrap();
    audit_sqlite.install_resources(&[&AuditLog::DEF]).await.unwrap();

    // 3. Build StoreRegistry routing by StoreTag
    let registry = StoreRegistry::new()
        .with_store::<PrimaryDb, _>(primary_sqlite.clone())
        .with_store::<AuditDb, _>(audit_sqlite.clone())
        .with_schema_support(primary_sqlite.clone())
        .with_schema_support(audit_sqlite.clone());

    let ctx = Context::new(registry);

    // Invariant 1: Resource metadata contains StoreTag reflection
    assert_eq!(Customer::DEF.store_name(), "PrimaryDb");
    assert_eq!(Customer::DEF.store_type_id(), TypeId::of::<PrimaryDb>());
    assert_eq!(AuditLog::DEF.store_name(), "AuditDb");
    assert_eq!(AuditLog::DEF.store_type_id(), TypeId::of::<AuditDb>());

    // Invariant 2: Customer creates only in primary_sqlite
    let cust = Customer::create(&ctx)
        .email("alice@example.com")
        .name("Alice")
        .call()
        .await
        .unwrap();

    assert_eq!(cust.email, "alice@example.com");

    let loaded = Customer::query(&ctx).all().await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].id, cust.id);

    // Direct check on primary db
    let direct_customers = Customer::query(&Context::new(primary_sqlite.clone()))
        .all()
        .await
        .unwrap();
    assert_eq!(direct_customers.len(), 1);

    // Audit db does NOT contain customers table
    let audit_query = Customer::query(&Context::new(audit_sqlite.clone())).all().await;
    assert!(audit_query.is_err(), "Customer table should not exist in audit db");

    // Invariant 3: AuditLog creates only in audit_sqlite
    let audit = AuditLog::create(&ctx)
        .action("CustomerCreated")
        .call()
        .await
        .unwrap();

    assert_eq!(audit.action, "CustomerCreated");

    let direct_audits = AuditLog::query(&Context::new(audit_sqlite.clone()))
        .all()
        .await
        .unwrap();
    assert_eq!(direct_audits.len(), 1);

    let primary_query = AuditLog::query(&Context::new(primary_sqlite.clone())).all().await;
    assert!(primary_query.is_err(), "AuditLog table should not exist in primary db");

    // Invariant 4: Multi pipeline spanning multiple stores atomically
    ctx.multi()
        .create("new_customer", Customer::create(&ctx).email("bob@example.com").name("Bob"))
        .create_from("log_entry", |ctx, res| {
            let c: &Customer = res.get("new_customer").unwrap();
            AuditLog::create(ctx)
                .action(format!("Registered {}", c.email))
                .changeset()
        })
        .commit()
        .await
        .unwrap();

    let all_customers = Customer::query(&ctx).all().await.unwrap();
    assert_eq!(all_customers.len(), 2);

    let all_logs = AuditLog::query(&ctx).all().await.unwrap();
    assert_eq!(all_logs.len(), 2);
}

// ============================================================================
// Test 2: Mixed SQLite + Memory store routing
// ============================================================================

pub struct PersistentStore;
impl StoreTag for PersistentStore {}

pub struct EphemeralStore;
impl StoreTag for EphemeralStore {}

pub mod account_res {
    use super::*;

    resource! {
        Account {
        table "accounts";
        store PersistentStore;

        attributes {
            id: Uuid [pk];
            username: String;
        }

        actions {
            create create {
                primary;
                accept [username];
            }
            read read { primary; }
        }
    }}
}

pub mod session_res {
    use super::*;

    resource! {
        SessionToken {
        table "session_tokens";
        store EphemeralStore;

        attributes {
            id: Uuid [pk];
            token: String;
            account_id: Uuid;
        }

        actions {
            create create {
                primary;
                accept [token, account_id];
            }
            read read { primary; }
        }
    }}
}

pub use account_res::Account;
pub use session_res::SessionToken;

#[tokio::test]
async fn test_mixed_sqlite_and_memory_store_routing() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Account::DEF]).await.unwrap();
    let memory = Memory::new();

    let registry = StoreRegistry::new()
        .with_store::<PersistentStore, _>(sqlite)
        .with_store::<EphemeralStore, _>(memory);

    let ctx = Context::new(registry);

    let account = Account::create(&ctx).username("charlie").call().await.unwrap();
    let session = SessionToken::create(&ctx)
        .token("xyz_secret")
        .account_id(account.id)
        .call()
        .await
        .unwrap();

    assert_eq!(session.token, "xyz_secret");
    assert_eq!(session.account_id, account.id);

    let accounts = Account::query(&ctx).all().await.unwrap();
    assert_eq!(accounts.len(), 1);

    let sessions = SessionToken::query(&ctx).all().await.unwrap();
    assert_eq!(sessions.len(), 1);
}

// ============================================================================
// Test 3: Mock 3rd-party external data layer (zero ash-core changes)
// ============================================================================

#[derive(Clone, Default)]
struct MockKeyValueLayer {
    storage: Arc<Mutex<HashMap<Uuid, FieldMap>>>,
}

impl DataLayer for MockKeyValueLayer {
    async fn create(&self, _res: &ResourceDef, id: Uuid, fields: FieldMap) -> ash_core::Result<FieldMap> {
        let mut map = self.storage.lock().unwrap();
        map.insert(id, fields.clone());
        Ok(fields)
    }

    async fn update(&self, _res: &ResourceDef, id: Uuid, fields: FieldMap) -> ash_core::Result<FieldMap> {
        let mut map = self.storage.lock().unwrap();
        if let Some(existing) = map.get_mut(&id) {
            for (k, v) in fields.clone() {
                existing.insert(k, v);
            }
            Ok(existing.clone())
        } else {
            Err(Error::NotFound)
        }
    }

    async fn destroy(&self, _res: &ResourceDef, id: Uuid) -> ash_core::Result<()> {
        let mut map = self.storage.lock().unwrap();
        map.remove(&id);
        Ok(())
    }

    async fn run_query(&self, _res: &ResourceDef, _query: &CompiledQuery) -> ash_core::Result<Vec<FieldMap>> {
        let map = self.storage.lock().unwrap();
        Ok(map.values().cloned().collect())
    }

    async fn upsert(
        &self,
        res: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
        _id_def: &IdentityDef,
        _update_fields: &[String],
    ) -> ash_core::Result<FieldMap> {
        self.create(res, id, fields).await
    }
}

pub struct KeyValueDb;
impl StoreTag for KeyValueDb {}

pub mod kv_res {
    use super::*;

    resource! {
        KeyValueItem {
        table "kv_items";
        store KeyValueDb;

        attributes {
            id: Uuid [pk];
            key: String;
            value: String;
        }

        actions {
            create create {
                primary;
                accept [key, value];
            }
            read read { primary; }
            destroy destroy { primary; }
        }
    }}
}

pub use kv_res::KeyValueItem;

#[tokio::test]
async fn test_third_party_external_data_layer_mock() {
    let kv_store = MockKeyValueLayer::default();
    let registry = StoreRegistry::new().with_store::<KeyValueDb, _>(kv_store.clone());
    let ctx = Context::new(registry);

    let item = KeyValueItem::create(&ctx)
        .key("api_key")
        .value("secret_123")
        .call()
        .await
        .unwrap();

    assert_eq!(item.key, "api_key");
    assert_eq!(item.value, "secret_123");

    // Verify it stored in the mock layer
    assert_eq!(kv_store.storage.lock().unwrap().len(), 1);

    let queried = KeyValueItem::query(&ctx).all().await.unwrap();
    assert_eq!(queried.len(), 1);
    assert_eq!(queried[0].id, item.id);
}

// ============================================================================
// Test 4: Missing store diagnostic error
// ============================================================================

pub struct UnregisteredDb;
impl StoreTag for UnregisteredDb {}

pub mod orphan_res {
    use super::*;

    resource! {
        Orphan {
        table "orphans";
        store UnregisteredDb;

        attributes {
            id: Uuid [pk];
            name: String;
        }

        actions {
            create create {
                primary;
                accept [name];
            }
        }
    }}
}

pub use orphan_res::Orphan;

#[tokio::test]
async fn test_missing_store_diagnostic_error() {
    // Registry without UnregisteredDb and without default
    let registry = StoreRegistry::new().with_store::<PrimaryDb, _>(Memory::new());
    let ctx = Context::new(registry);

    let err = Orphan::create(&ctx).name("lonely").call().await.unwrap_err();

    let err_msg = err.to_string();
    assert!(
        err_msg.contains("no data layer registered for store `UnregisteredDb` on resource `Orphan`"),
        "expected clear diagnostic error, got: {err_msg}"
    );
}

// ============================================================================
// Test 5: DefaultStore fallback
// ============================================================================

pub mod default_res {
    use super::*;

    resource! {
        DefaultTarget {
        table "default_targets";

        attributes {
            id: Uuid [pk];
            content: String;
        }

        actions {
            create create {
                primary;
                accept [content];
            }
            read read { primary; }
        }
    }}
}

pub use default_res::DefaultTarget;

#[tokio::test]
async fn test_fallback_to_default_store() {
    let memory = Memory::new();
    let registry = StoreRegistry::new().with_default(memory);
    let ctx = Context::new(registry);

    assert_eq!(DefaultTarget::DEF.store_name(), "DefaultStore");

    let item = DefaultTarget::create(&ctx)
        .content("uses fallback")
        .call()
        .await
        .unwrap();

    assert_eq!(item.content, "uses fallback");

    let list = DefaultTarget::query(&ctx).all().await.unwrap();
    assert_eq!(list.len(), 1);
}

// ============================================================================
// Test 6: HasStore trait retrieval
// ============================================================================

#[tokio::test]
async fn test_has_store_trait_retrieval() {
    let memory = Memory::new();
    let registry = StoreRegistry::new().with_store::<PersistentStore, _>(memory);

    // Using HasStore<PersistentStore>
    let dyn_store: &dyn ash_core::DynDataLayer = HasStore::<PersistentStore>::get_store(&registry);
    let fields = dyn_store
        .run_query_dyn(&Account::DEF, &CompiledQuery::default())
        .await
        .unwrap();
    assert_eq!(fields.len(), 0);
}
