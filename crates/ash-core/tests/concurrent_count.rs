//! A counted page reads its count alongside the page, as Ash does, rather than one after
//! the other.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use ash_core::{CompiledQuery, Context, DataLayer, FieldMap, ResourceDef, Result, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    Note {
        table "notes";

        attributes {
            id: Uuid [pk];
            body: String;
        }

        actions {
            create create { primary; accept [body]; }
            read read { primary; }
        }
    }
}

/// Memory, with every read taking a while, noting the most reads ever under way at once.
#[derive(Clone, Default)]
struct Slow {
    inner: Memory,
    reading: Arc<AtomicUsize>,
    most: Arc<AtomicUsize>,
}

impl Slow {
    async fn read<T>(&self, read: impl Future<Output = T>) -> T {
        let now = self.reading.fetch_add(1, Ordering::SeqCst) + 1;
        self.most.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let out = read.await;
        self.reading.fetch_sub(1, Ordering::SeqCst);
        out
    }
}

impl DataLayer for Slow {
    async fn create(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value, fields: FieldMap) -> Result<FieldMap> {
        self.inner.create(resource, tenant, id, fields).await
    }

    async fn update(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value, fields: FieldMap) -> Result<FieldMap> {
        self.inner.update(resource, tenant, id, fields).await
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: ash_core::Value) -> Result<()> {
        self.inner.destroy(resource, tenant, id).await
    }

    async fn run_query(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<Vec<FieldMap>> {
        self.read(self.inner.run_query(resource, query)).await
    }

    async fn count(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<usize> {
        self.read(self.inner.count(resource, query)).await
    }
}

#[tokio::test]
async fn a_counted_page_reads_its_count_alongside_the_page() {
    let slow = Slow::default();
    let ctx = Context::new(slow.clone());
    for i in 0..5 {
        Note::create(&ctx).body(format!("note {i}")).await.unwrap();
    }

    let page = Note::query(&ctx).page_offset(2, 0, true).await.unwrap();
    assert_eq!((page.results.len(), page.total_count), (2, Some(5)));
    assert_eq!(slow.most.load(Ordering::SeqCst), 2, "the count and the page are read at once");
}
