//! A bulk destroy under an optimistic lock, with another writer moving a record on between
//! the bulk action's read and its write: the stale record is left out, as Ash's bulk
//! destroy leaves it out, before anything runs for it. Its related records stay, and the
//! other records go as they would.

use std::sync::{Arc, Mutex};

use ash_core::{
    AtomicCondition, AtomicUpdate, BulkDestroyOptions, BulkTransaction, CompiledQuery, Context, DataLayer, FieldMap, ResourceDef,
    Result, TransactionSupport, Value, resource,
};
use ash_memory::Memory;
use uuid::Uuid;

/// The write another writer makes: the resource, the record and what it stores.
type Race = (&'static str, Value, FieldMap);

/// A memory data layer where, once armed, reading `resource` lets another writer store
/// `fields` on record `id` just after: the reader holds the record as it was.
#[derive(Clone)]
struct Racing {
    inner: Memory,
    race: Arc<Mutex<Option<Race>>>,
}

impl Racing {
    fn new() -> Self {
        Self { inner: Memory::new(), race: Arc::default() }
    }

    fn arm(&self, resource: &'static str, id: Uuid, fields: FieldMap) {
        *self.race.lock().unwrap() = Some((resource, Value::from(id), fields));
    }
}

impl DataLayer for Racing {
    async fn create(&self, resource: &ResourceDef, tenant: Option<&str>, id: Value, fields: FieldMap) -> Result<FieldMap> {
        self.inner.create(resource, tenant, id, fields).await
    }

    async fn update(&self, resource: &ResourceDef, tenant: Option<&str>, id: Value, fields: FieldMap) -> Result<FieldMap> {
        self.inner.update(resource, tenant, id, fields).await
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: Value) -> Result<()> {
        self.inner.destroy(resource, tenant, id).await
    }

    async fn run_query(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<Vec<FieldMap>> {
        let rows = self.inner.run_query(resource, query).await?;
        let race = {
            let mut race = self.race.lock().unwrap();
            if race.as_ref().is_some_and(|(name, _, _)| *name == resource.name) { race.take() } else { None }
        };
        if let Some((_, id, fields)) = race {
            self.inner.update(resource, None, id, fields).await?;
        }
        Ok(rows)
    }

    fn can_update_atomically(&self, resource: &ResourceDef) -> bool {
        self.inner.can_update_atomically(resource)
    }

    async fn update_atomic(&self, resource: &ResourceDef, query: &CompiledQuery, update: &AtomicUpdate) -> Result<Vec<FieldMap>> {
        self.inner.update_atomic(resource, query, update).await
    }

    fn can_destroy_atomically(&self, resource: &ResourceDef) -> bool {
        self.inner.can_destroy_atomically(resource)
    }

    async fn destroy_atomic(&self, resource: &ResourceDef, query: &CompiledQuery, conditions: &[AtomicCondition]) -> Result<Vec<FieldMap>> {
        self.inner.destroy_atomic(resource, query, conditions).await
    }

    async fn count(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<usize> {
        self.inner.count(resource, query).await
    }
}

impl TransactionSupport for Racing {
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        f(self).await
    }
}

pub mod ledger {
    use super::*;

    resource! {
        Ledger {
            table "lock_race_ledgers";

            attributes {
                id: Uuid [pk];
                name: String;
                version: i64 [default: 1];
                archived_at: Option<String>;
            }

            relationships {
                has_many entries: super::entry::Entry [fk: ledger_id, on_delete: cascade];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
                update rename { change optimistic_lock(version); accept [name]; }
                destroy remove { primary; change optimistic_lock(version); }
                destroy archive {
                    soft;
                    change optimistic_lock(version);
                    change set(archived_at = ash_core::utc_now_iso8601());
                }
            }
        }
    }
}

pub mod entry {
    use super::*;

    resource! {
        Entry {
            table "lock_race_entries";

            attributes {
                id: Uuid [pk];
                ledger_id: Uuid;
                amount: i64;
            }

            relationships {
                belongs_to ledger: super::ledger::Ledger [fk: ledger_id];
            }

            actions {
                create create { primary; accept [ledger_id, amount]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}

use entry::Entry;
use ledger::Ledger;

fn moved_on() -> FieldMap {
    FieldMap::from([("version".to_string(), Value::Int(2))])
}

#[tokio::test]
async fn a_stale_record_keeps_its_related_records() {
    let data = Racing::new();
    let ctx = Context::new(data.clone());
    let ledger = Ledger::create(&ctx).name("Main").await.unwrap();
    let entry = Entry::create(&ctx).ledger_id(ledger.id).amount(5).await.unwrap();

    data.arm("Ledger", ledger.id, moved_on());
    let result = ash_core::bulk_destroy::<Ledger, _, _>(&ctx, "remove", &[ledger.id], BulkDestroyOptions::default()).await.unwrap();
    assert_eq!((result.count, result.error_count), (0, 0), "the stale record is left out, not failed");
    assert_eq!(Ledger::get(&ctx, ledger.id).await.unwrap().version, 2, "it stays");
    assert!(Entry::get(&ctx, entry.id).await.is_ok(), "and so do its entries");
}

#[tokio::test]
async fn a_stale_record_is_left_out_of_a_soft_destroy() {
    for transaction in [BulkTransaction::Batch, BulkTransaction::All, BulkTransaction::Off] {
        for stop_on_error in [false, true] {
            let data = Racing::new();
            let ctx = Context::new(data.clone());
            let stale = Ledger::create(&ctx).name("Stale").await.unwrap();
            let fresh = Ledger::create(&ctx).name("Fresh").await.unwrap();

            data.arm("Ledger", stale.id, moved_on());
            let opts = BulkDestroyOptions { transaction, stop_on_error, ..BulkDestroyOptions::default() };
            let result = ash_core::bulk_destroy::<Ledger, _, _>(&ctx, "archive", &[stale.id, fresh.id], opts).await.unwrap();
            let case = format!("{transaction:?}, stop_on_error {stop_on_error}");
            assert_eq!((result.count, result.error_count), (1, 0), "{case}");
            let (stale, fresh) = (Ledger::get(&ctx, stale.id).await.unwrap(), Ledger::get(&ctx, fresh.id).await.unwrap());
            assert!(stale.archived_at.is_none(), "{case}: the stale record isn't archived");
            assert!(fresh.archived_at.is_some(), "{case}: the other is");
        }
    }
}
