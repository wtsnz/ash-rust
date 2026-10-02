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
    /// The attributes to read, as Ash's `select`: `None` reads every one. The primary key
    /// is always read. A record read with a selection lacks the attributes left out.
    pub select: Option<Vec<String>>,
    pub calculations: Vec<String>,
    pub calculation_args: std::collections::HashMap<String, FieldMap>,
    pub aggregates: Vec<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub tenant: Option<String>,
}

impl CompiledQuery {
    /// Whether the query reads `attribute`: it's selected, or the primary key, or the
    /// query selects everything.
    pub fn reads(&self, attribute: &crate::resource::AttributeDef) -> bool {
        attribute.primary_key
            || self
                .select
                .as_ref()
                .is_none_or(|select| select.iter().any(|name| name == attribute.name))
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid Ash DataLayer",
    label = "not a DataLayer",
    note = "use Memory, Sqlite, Postgres, or implement DataLayer"
)]
pub trait DataLayer: Send + Sync {
    /// Writes a new record. `tenant` is the context's: a data layer that keeps
    /// context-tenant resources apart (a Postgres schema, a memory table per tenant) writes
    /// it there, as Ash's data layers do with a changeset's tenant.
    fn create(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send;

    fn update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
        fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send;

    fn destroy(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        id: Uuid,
    ) -> impl Future<Output = Result<()>> + Send;

    fn run_query(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send;

    /// Whether this data layer runs updates as one statement, checking conditions and
    /// raising their errors within it, as Ash's data layers that can `update_query` and
    /// `expr_error`. One that can't has updates read their record first.
    fn can_update_atomically(&self, _resource: &ResourceDef) -> bool {
        false
    }

    /// Updates the records `query` selects as `update` says, in one statement: checks
    /// each record against the update's conditions, in order, failing with the first
    /// that holds, then sets the update's values, every one computed from the record as
    /// it was. Returns the updated records.
    fn update_atomic(
        &self,
        resource: &ResourceDef,
        _query: &CompiledQuery,
        _update: &crate::atomic::AtomicUpdate,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "this data layer can't update {} atomically",
            resource.name
        ))))
    }

    /// Whether this data layer runs destroys as one statement, checking conditions and
    /// raising their errors within it, as Ash's data layers that can `destroy_query` and
    /// `expr_error`. One that can't has destroys read their record first.
    fn can_destroy_atomically(&self, _resource: &ResourceDef) -> bool {
        false
    }

    /// Deletes the records `query` selects in one statement: checks each record against
    /// `conditions`, in order, failing with the first that holds, then deletes them all.
    /// Returns the deleted records.
    fn destroy_atomic(
        &self,
        resource: &ResourceDef,
        _query: &CompiledQuery,
        _conditions: &[crate::atomic::AtomicCondition],
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "this data layer can't destroy {} atomically",
            resource.name
        ))))
    }

    /// How many records `query` would return, as Ash's data layers count with an
    /// aggregate query. A data layer that can count without reading the records (a SQL
    /// `COUNT(*)`) should; by default it reads them.
    fn count(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
    ) -> impl Future<Output = Result<usize>> + Send {
        async move { Ok(self.run_query(resource, query).await?.len()) }
    }

    fn upsert(
        &self,
        resource: &ResourceDef,
        _tenant: Option<&str>,
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
        tenant: Option<&str>,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        async move {
            let mut results = Vec::with_capacity(rows.len());
            for (id, fields) in rows {
                results.push(self.create(resource, tenant, id, fields).await?);
            }
            Ok(results)
        }
    }

    fn bulk_destroy(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        ids: &[Uuid],
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            for id in ids {
                self.destroy(resource, tenant, *id).await?;
            }
            Ok(())
        }
    }

    /// Writes several updates together: each row's id and the attributes it changes, as
    /// [`update`](Self::update) takes them. Each row has its own result, in order, so a
    /// row that's gone fails alone; the outer error is for the batch as a whole. A data
    /// layer that can write the batch at once (one statement, one round trip) should.
    fn bulk_update(
        &self,
        resource: &ResourceDef,
        tenant: Option<&str>,
        rows: Vec<(Uuid, FieldMap)>,
    ) -> impl Future<Output = Result<Vec<Result<FieldMap>>>> + Send {
        async move {
            let mut results = Vec::with_capacity(rows.len());
            for (id, fields) in rows {
                results.push(self.update(resource, tenant, id, fields).await);
            }
            Ok(results)
        }
    }
}

/// A data layer that stores nothing; every operation fails.
///
/// `resource!` uses it for the context behind `Resource::build_<action>()`, which builds a
/// record without persisting it, so a crate needs no real data layer to define resources.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDataLayer;

fn no_data_layer<T>(resource: &ResourceDef) -> std::future::Ready<Result<T>> {
    std::future::ready(Err(Error::DataLayer(format!(
        "{} has no data layer here; use a Context with a real data layer to persist it",
        resource.name
    ))))
}

impl DataLayer for NoDataLayer {
    fn create(
        &self,
        resource: &ResourceDef,
        _tenant: Option<&str>,
        _id: Uuid,
        _fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(resource)
    }

    fn update(
        &self,
        resource: &ResourceDef,
        _tenant: Option<&str>,
        _id: Uuid,
        _fields: FieldMap,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(resource)
    }

    fn destroy(
        &self,
        resource: &ResourceDef,
        _tenant: Option<&str>,
        _id: Uuid,
    ) -> impl Future<Output = Result<()>> + Send {
        no_data_layer(resource)
    }

    fn run_query(
        &self,
        resource: &ResourceDef,
        _query: &CompiledQuery,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        no_data_layer(resource)
    }

    fn upsert(
        &self,
        resource: &ResourceDef,
        _tenant: Option<&str>,
        _id: Uuid,
        _fields: FieldMap,
        _identity: &IdentityDef,
        _update_fields: &[String],
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(resource)
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
