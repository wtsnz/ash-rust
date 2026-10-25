use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::data_layer::{DataLayer, SchemaSupport, TransactionSupport};
use crate::error::{Error, Result};
use crate::resource::{DataLayerKind, Resource, ResourceDef};
use crate::store::{HasStore, StoreTag};
use crate::value::FieldMap;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type-erased object-safe interface for any [`DataLayer`].
///
/// ```compile_fail
/// async fn raw(layer: &dyn ash_core::DynDataLayer, resource: &ash_core::ResourceDef) {
///     layer.run_query_dyn(resource, &ash_core::CompiledQuery::default()).await;
/// }
/// ```
pub trait DynDataLayer: Send + Sync {
    fn create_dyn<'a>(&'a self, plan: &'a crate::CreatePlan<'a>)
    -> BoxFuture<'a, Result<FieldMap>>;
    fn update_dyn<'a>(&'a self, plan: &'a crate::UpdatePlan<'a>)
    -> BoxFuture<'a, Result<FieldMap>>;
    fn destroy_dyn<'a>(&'a self, plan: &'a crate::DestroyPlan<'a>) -> BoxFuture<'a, Result<()>>;
    fn run_query_dyn<'a>(
        &'a self,
        plan: &'a crate::QueryPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;
    fn count_dyn<'a>(&'a self, plan: &'a crate::CountPlan<'a>) -> BoxFuture<'a, Result<usize>>;
    fn run_query_with_count_dyn<'a>(
        &'a self,
        plan: &'a crate::QueryWithCountPlan<'a>,
    ) -> BoxFuture<'a, Result<(Vec<FieldMap>, usize)>>;
    fn run_query_per_key_dyn<'a>(
        &'a self,
        plan: &'a crate::PerKeyPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<Vec<FieldMap>>>>;
    fn update_atomic_dyn<'a>(
        &'a self,
        plan: &'a crate::AtomicUpdatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;
    fn destroy_atomic_dyn<'a>(
        &'a self,
        plan: &'a crate::AtomicDestroyPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;
    fn bulk_create_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkCreatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;
    fn bulk_update_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkUpdatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<Result<FieldMap>>>>;
    fn bulk_destroy_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkDestroyPlan<'a>,
    ) -> BoxFuture<'a, Result<()>>;
    fn upsert_dyn<'a>(&'a self, plan: &'a crate::UpsertPlan<'a>)
    -> BoxFuture<'a, Result<FieldMap>>;
    fn can_update_atomically_dyn(&self, resource: &ResourceDef) -> bool;
    fn can_raise_atomically_dyn(&self, resource: &ResourceDef) -> bool;
    fn can_destroy_atomically_dyn(&self, resource: &ResourceDef) -> bool;
    fn can_run_query_per_key_dyn(&self, resource: &ResourceDef, by: &crate::PerKey<'_>) -> bool;
}

impl<T: DataLayer> DynDataLayer for T {
    fn create_dyn<'a>(
        &'a self,
        plan: &'a crate::CreatePlan<'a>,
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.create(plan))
    }
    fn update_dyn<'a>(
        &'a self,
        plan: &'a crate::UpdatePlan<'a>,
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.update(plan))
    }
    fn destroy_dyn<'a>(&'a self, plan: &'a crate::DestroyPlan<'a>) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.destroy(plan))
    }
    fn run_query_dyn<'a>(
        &'a self,
        plan: &'a crate::QueryPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.run_query(plan))
    }
    fn count_dyn<'a>(&'a self, plan: &'a crate::CountPlan<'a>) -> BoxFuture<'a, Result<usize>> {
        Box::pin(self.count(plan))
    }
    fn run_query_with_count_dyn<'a>(
        &'a self,
        plan: &'a crate::QueryWithCountPlan<'a>,
    ) -> BoxFuture<'a, Result<(Vec<FieldMap>, usize)>> {
        Box::pin(self.run_query_with_count(plan))
    }
    fn run_query_per_key_dyn<'a>(
        &'a self,
        plan: &'a crate::PerKeyPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<Vec<FieldMap>>>> {
        Box::pin(self.run_query_per_key(plan))
    }
    fn update_atomic_dyn<'a>(
        &'a self,
        plan: &'a crate::AtomicUpdatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.update_atomic(plan))
    }
    fn destroy_atomic_dyn<'a>(
        &'a self,
        plan: &'a crate::AtomicDestroyPlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.destroy_atomic(plan))
    }
    fn bulk_create_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkCreatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.bulk_create(plan))
    }
    fn bulk_update_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkUpdatePlan<'a>,
    ) -> BoxFuture<'a, Result<Vec<Result<FieldMap>>>> {
        Box::pin(self.bulk_update(plan))
    }
    fn bulk_destroy_dyn<'a>(
        &'a self,
        plan: &'a crate::BulkDestroyPlan<'a>,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.bulk_destroy(plan))
    }
    fn upsert_dyn<'a>(
        &'a self,
        plan: &'a crate::UpsertPlan<'a>,
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.upsert(plan))
    }
    fn can_update_atomically_dyn(&self, resource: &ResourceDef) -> bool {
        self.can_update_atomically(resource)
    }
    fn can_raise_atomically_dyn(&self, resource: &ResourceDef) -> bool {
        self.can_raise_atomically(resource)
    }
    fn can_destroy_atomically_dyn(&self, resource: &ResourceDef) -> bool {
        self.can_destroy_atomically(resource)
    }
    fn can_run_query_per_key_dyn(&self, resource: &ResourceDef, by: &crate::PerKey<'_>) -> bool {
        self.can_run_query_per_key(resource, by)
    }
}

/// Type-erased object-safe interface for any [`SchemaSupport`].
pub trait DynSchemaSupport: Send + Sync {
    fn install_resources_dyn<'a>(
        &'a self,
        resources: &'a [&'a ResourceDef],
    ) -> BoxFuture<'a, Result<()>>;
}

impl<T: SchemaSupport> DynSchemaSupport for T {
    fn install_resources_dyn<'a>(
        &'a self,
        resources: &'a [&'a ResourceDef],
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.install_resources(resources))
    }
}

/// A type-safe multi-store data layer registry that routes operations to the appropriate
/// backend based on static [`StoreTag`] types, resource configuration, or explicit registration.
#[derive(Clone, Default)]
pub struct StoreRegistry {
    stores: HashMap<TypeId, Arc<dyn DynDataLayer>>,
    default: Option<Arc<dyn DynDataLayer>>,
    by_kind: HashMap<DataLayerKind, Arc<dyn DynDataLayer>>,
    by_resource: HashMap<String, Arc<dyn DynDataLayer>>,
    schema_supporters: Vec<Arc<dyn DynSchemaSupport>>,
}

/// Backward-compatible alias for [`StoreRegistry`].
pub type DataLayerRegistry = StoreRegistry;

impl StoreRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a data layer for a specific [`StoreTag`].
    pub fn with_store<T: StoreTag, D: DataLayer + 'static>(mut self, data: D) -> Self {
        self.stores.insert(TypeId::of::<T>(), Arc::new(data));
        self
    }

    /// Register a type-erased data layer for a specific [`StoreTag`].
    pub fn with_store_arc<T: StoreTag>(mut self, data: Arc<dyn DynDataLayer>) -> Self {
        self.stores.insert(TypeId::of::<T>(), data);
        self
    }

    /// Set the fallback data layer for resources without a specific mapping.
    pub fn with_default<D: DataLayer + 'static>(mut self, data: D) -> Self {
        self.default = Some(Arc::new(data));
        self
    }

    /// Set the fallback data layer from an Arc.
    pub fn with_default_arc(mut self, data: Arc<dyn DynDataLayer>) -> Self {
        self.default = Some(data);
        self
    }

    /// Register a data layer for a specific [`DataLayerKind`].
    pub fn register_kind<D: DataLayer + 'static>(mut self, kind: DataLayerKind, data: D) -> Self {
        self.by_kind.insert(kind, Arc::new(data));
        self
    }

    /// Register a data layer for a specific resource by name.
    pub fn register_resource<D: DataLayer + 'static>(
        mut self,
        resource_name: impl Into<String>,
        data: D,
    ) -> Self {
        self.by_resource
            .insert(resource_name.into(), Arc::new(data));
        self
    }

    /// Register a schema supporter (e.g. SQLite database) for automatic migration/table creation.
    pub fn with_schema_support<S: SchemaSupport + 'static>(mut self, supporter: S) -> Self {
        self.schema_supporters.push(Arc::new(supporter));
        self
    }

    /// Retrieve the data layer for a specific [`Resource`] type.
    pub fn get_layer_for<R: Resource>(&self) -> Result<&dyn DynDataLayer> {
        self.get_layer(&R::DEF)
    }

    /// Look up the data layer responsible for the given resource.
    pub fn get_layer<'a>(&'a self, resource: &ResourceDef) -> Result<&'a dyn DynDataLayer> {
        if let Some(layer) = self.by_resource.get(resource.name) {
            return Ok(&**layer);
        }
        let store_tid = resource.store_type_id();
        if let Some(layer) = self.stores.get(&store_tid) {
            return Ok(&**layer);
        }
        if let Some(layer) = self.by_kind.get(&resource.data_layer) {
            return Ok(&**layer);
        }
        if let Some(layer) = &self.default {
            return Ok(&**layer);
        }
        Err(Error::Invalid(format!(
            "no data layer registered for store `{}` on resource `{}`",
            resource.store_name, resource.name
        )))
    }
}

impl<T: StoreTag> HasStore<T> for StoreRegistry {
    fn get_store(&self) -> &dyn DynDataLayer {
        let tid = TypeId::of::<T>();
        if let Some(layer) = self.stores.get(&tid) {
            return &**layer;
        }
        if let Some(layer) = &self.default {
            return &**layer;
        }
        panic!(
            "no data layer registered for store `{}`",
            std::any::type_name::<T>()
        );
    }
}

impl std::fmt::Debug for StoreRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreRegistry")
            .field("has_default", &self.default.is_some())
            .field("stores_count", &self.stores.len())
            .field("by_kind_count", &self.by_kind.len())
            .field("by_resource_count", &self.by_resource.len())
            .finish()
    }
}

impl DataLayer for StoreRegistry {
    async fn create(&self, plan: &crate::CreatePlan<'_>) -> Result<FieldMap> {
        self.get_layer(plan.resource())?.create_dyn(plan).await
    }
    async fn update(&self, plan: &crate::UpdatePlan<'_>) -> Result<FieldMap> {
        self.get_layer(plan.resource())?.update_dyn(plan).await
    }
    async fn destroy(&self, plan: &crate::DestroyPlan<'_>) -> Result<()> {
        self.get_layer(plan.resource())?.destroy_dyn(plan).await
    }
    async fn run_query(&self, plan: &crate::QueryPlan<'_>) -> Result<Vec<FieldMap>> {
        self.get_layer(plan.resource())?.run_query_dyn(plan).await
    }
    async fn count(&self, plan: &crate::CountPlan<'_>) -> Result<usize> {
        self.get_layer(plan.resource())?.count_dyn(plan).await
    }
    async fn run_query_with_count(
        &self,
        plan: &crate::QueryWithCountPlan<'_>,
    ) -> Result<(Vec<FieldMap>, usize)> {
        self.get_layer(plan.resource())?
            .run_query_with_count_dyn(plan)
            .await
    }
    async fn run_query_per_key(&self, plan: &crate::PerKeyPlan<'_>) -> Result<Vec<Vec<FieldMap>>> {
        self.get_layer(plan.resource())?
            .run_query_per_key_dyn(plan)
            .await
    }
    async fn update_atomic(&self, plan: &crate::AtomicUpdatePlan<'_>) -> Result<Vec<FieldMap>> {
        self.get_layer(plan.resource())?
            .update_atomic_dyn(plan)
            .await
    }
    async fn destroy_atomic(&self, plan: &crate::AtomicDestroyPlan<'_>) -> Result<Vec<FieldMap>> {
        self.get_layer(plan.resource())?
            .destroy_atomic_dyn(plan)
            .await
    }
    async fn bulk_create(&self, plan: &crate::BulkCreatePlan<'_>) -> Result<Vec<FieldMap>> {
        self.get_layer(plan.resource())?.bulk_create_dyn(plan).await
    }
    async fn bulk_update(&self, plan: &crate::BulkUpdatePlan<'_>) -> Result<Vec<Result<FieldMap>>> {
        self.get_layer(plan.resource())?.bulk_update_dyn(plan).await
    }
    async fn bulk_destroy(&self, plan: &crate::BulkDestroyPlan<'_>) -> Result<()> {
        self.get_layer(plan.resource())?
            .bulk_destroy_dyn(plan)
            .await
    }
    async fn upsert(&self, plan: &crate::UpsertPlan<'_>) -> Result<FieldMap> {
        self.get_layer(plan.resource())?.upsert_dyn(plan).await
    }
    fn can_update_atomically(&self, resource: &ResourceDef) -> bool {
        self.get_layer(resource)
            .is_ok_and(|layer| layer.can_update_atomically_dyn(resource))
    }
    fn can_raise_atomically(&self, resource: &ResourceDef) -> bool {
        self.get_layer(resource)
            .is_ok_and(|layer| layer.can_raise_atomically_dyn(resource))
    }
    fn can_destroy_atomically(&self, resource: &ResourceDef) -> bool {
        self.get_layer(resource)
            .is_ok_and(|layer| layer.can_destroy_atomically_dyn(resource))
    }
    /// A join resource in another store can't join in the same statement.
    fn can_run_query_per_key(
        &self,
        resource: &ResourceDef,
        by: &crate::data_layer::PerKey<'_>,
    ) -> bool {
        let Ok(layer) = self.get_layer(resource) else {
            return false;
        };
        if let crate::data_layer::PerKey::Through {
            resource: through, ..
        } = by
        {
            match self.get_layer(through) {
                Ok(other) if std::ptr::addr_eq(layer, other) => {}
                _ => return false,
            }
        }
        layer.can_run_query_per_key_dyn(resource, by)
    }
}

impl SchemaSupport for StoreRegistry {
    async fn install_resources(&self, resources: &[&ResourceDef]) -> Result<()> {
        for supporter in &self.schema_supporters {
            supporter.install_resources_dyn(resources).await?;
        }
        Ok(())
    }
}

impl TransactionSupport for StoreRegistry {
    async fn transaction<F, Fut, T>(&self, _f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        Err(Error::AtomicityUnsupported)
    }

    /// Its stores each keep their own transactions, which this doesn't open, so a bulk
    /// action's batches run as they are.
    fn can_transact(&self) -> bool {
        false
    }
}
