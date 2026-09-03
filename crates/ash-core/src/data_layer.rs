use uuid::Uuid;

use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::{IdentityDef, ResourceDef};
use crate::value::FieldMap;

#[derive(Clone, Debug, Default)]
pub struct Sort {
    pub field: String,
    pub descending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CompiledQuery {
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    pub calculations: Vec<String>,
    pub aggregates: Vec<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub tenant: Option<String>,
}

pub trait DataLayer: Send + Sync {
    fn create(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send;

    fn update(
        &self,
        resource: &ResourceDef,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send;

    fn destroy(&self, resource: &ResourceDef, id: Uuid) -> impl Future<Output = Result<()>> + Send;

    fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send;

    fn upsert(
        &self,
        resource: &ResourceDef,
        _id: Uuid,
        _fields: FieldMap,
        _identity: &IdentityDef,
        _update_fields: &[String],
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "upsert is not supported for resource `{}` by this data layer",
            resource.name
        ))))
    }

    fn bulk_create(
        &self,
        resource: &ResourceDef,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        async move {
            let mut results = Vec::with_capacity(rows.len());
            for (id, fields) in rows {
                results.push(self.create(resource, id, fields).await?);
            }
            Ok(results)
        }
    }

    fn bulk_destroy(
        &self,
        resource: &ResourceDef,
        ids: &[Uuid],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            for id in ids {
                self.destroy(resource, *id).await?;
            }
            Ok(())
        }
    }
}

pub trait SchemaSupport: Send + Sync {
    fn install_resources(
        &self,
        resources: &[&ResourceDef],
    ) -> impl Future<Output = Result<()>> + Send;
}

pub trait TransactionSupport: DataLayer + Clone {
    fn transaction<F, Fut, T>(&self, f: F) -> impl Future<Output = Result<T>> + Send
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send;
}
