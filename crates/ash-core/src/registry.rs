use std::any::TypeId;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use uuid::Uuid;

use crate::data_layer::{CompiledQuery, DataLayer, SchemaSupport, TransactionSupport};
use crate::error::{Error, Result};
use crate::resource::{DataLayerKind, IdentityDef, Resource, ResourceDef};
use crate::store::{HasStore, StoreTag};
use crate::value::FieldMap;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Type-erased object-safe interface for any [`DataLayer`].
pub trait DynDataLayer: Send + Sync {
    fn create_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> BoxFuture<'a, Result<FieldMap>>;

    fn update_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> BoxFuture<'a, Result<FieldMap>>;

    fn destroy_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
    ) -> BoxFuture<'a, Result<()>>;

    fn run_query_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;

    fn upsert_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
        identity: &'a IdentityDef,
        update_fields: &'a [String],
    ) -> BoxFuture<'a, Result<FieldMap>>;

    fn bulk_create_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>>;

    fn bulk_destroy_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<()>>;
}

impl<T: DataLayer> DynDataLayer for T {
    fn create_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.create(resource, id, fields))
    }

    fn update_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.update(resource, id, fields))
    }

    fn destroy_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.destroy(resource, id))
    }

    fn run_query_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.run_query(resource, query))
    }

    fn upsert_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        id: Uuid,
        fields: FieldMap,
        identity: &'a IdentityDef,
        update_fields: &'a [String],
    ) -> BoxFuture<'a, Result<FieldMap>> {
        Box::pin(self.upsert(resource, id, fields, identity, update_fields))
    }

    fn bulk_create_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> BoxFuture<'a, Result<Vec<FieldMap>>> {
        Box::pin(self.bulk_create(resource, rows))
    }

    fn bulk_destroy_dyn<'a>(
        &'a self,
        resource: &'a ResourceDef,
        ids: &'a [Uuid],
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(self.bulk_destroy(resource, ids))
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
        self.by_resource.insert(resource_name.into(), Arc::new(data));
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
    async fn create(&self, resource: &ResourceDef, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        let layer = self.get_layer(resource)?;
        layer.create_dyn(resource, id, fields).await
    }

    async fn update(&self, resource: &ResourceDef, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        let layer = self.get_layer(resource)?;
        layer.update_dyn(resource, id, fields).await
    }

    async fn destroy(&self, resource: &ResourceDef, id: Uuid) -> Result<()> {
        let layer = self.get_layer(resource)?;
        layer.destroy_dyn(resource, id).await
    }

    async fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> Result<Vec<FieldMap>> {
        let layer = self.get_layer(resource)?;
        layer.run_query_dyn(resource, query).await
    }

    async fn upsert(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
        identity: &IdentityDef,
        update_fields: &[String],
    ) -> Result<FieldMap> {
        let layer = self.get_layer(resource)?;
        layer
            .upsert_dyn(resource, id, fields, identity, update_fields)
            .await
    }

    async fn bulk_create(
        &self,
        resource: &ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> Result<Vec<FieldMap>> {
        let layer = self.get_layer(resource)?;
        layer.bulk_create_dyn(resource, rows).await
    }

    async fn bulk_destroy(&self, resource: &ResourceDef, ids: &[Uuid]) -> Result<()> {
        let layer = self.get_layer(resource)?;
        layer.bulk_destroy_dyn(resource, ids).await
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
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        f(self).await
    }
}
