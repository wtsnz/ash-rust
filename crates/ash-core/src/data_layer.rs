use crate::error::{Error, Result};
use crate::filter::Filter;
use crate::resource::ResourceDef;
use crate::value::FieldMap;

#[derive(Clone, Debug, Default)]
pub struct Sort {
    pub field: String,
    pub descending: bool,
    /// Where the field may be read, when a field policy hides it from the actor: it sorts
    /// as null elsewhere, as Ash sorts a client's sort on it (`if <policy> then field else
    /// nil`). See [`guard_input_sort`](crate::guard_input_sort).
    pub guard: Option<Filter>,
    /// Where nulls go: first, or last. `None`: as Ash places them, last ascending and
    /// first descending.
    pub nulls_first: Option<bool>,
    /// Use projected aggregate operands, as client sorts do. Trusted preparations
    /// leave this false and may order by stored derived values.
    pub projected: bool,
}

impl Sort {
    /// Whether nulls sort first.
    pub fn nulls_first(&self) -> bool {
        self.nulls_first.unwrap_or(self.descending)
    }

    /// This sort, the other way around: a backward page reads in it.
    pub fn reversed(&self) -> Sort {
        Sort {
            descending: !self.descending,
            nulls_first: self.nulls_first.map(|first| !first),
            ..self.clone()
        }
    }
}

/// Inert compiler input, also available to migration tooling. It cannot execute on
/// a data layer without an opaque core-built plan.
#[derive(Clone, Debug, Default)]
pub struct CompiledQuery {
    pub lock: Option<crate::ReadLock>,
    pub distinct_on: Option<crate::DistinctOn>,
    pub filter: Option<Filter>,
    pub sort: Vec<Sort>,
    /// The attributes to read, as Ash's `select`: `None` reads every one. The primary key
    /// is always read. A record read with a selection lacks the attributes left out.
    pub select: Option<Vec<String>>,
    /// The actor the read runs as. Aggregates count only the related rows the actor may
    /// read, as Ash authorizes an aggregate's query by default.
    pub actor: Option<crate::actor::Actor>,
    pub calculations: Vec<String>,
    pub calculation_args: std::collections::HashMap<String, FieldMap>,
    pub aggregates: Vec<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub tenant: Option<String>,
}

/// How a row of a relationship's destination relates to the key it's read for.
#[derive(Clone, Copy, Debug)]
pub enum PerKey<'a> {
    /// The row's own attribute holds the key: has_many, has_one, belongs_to.
    Attribute(&'a str),
    /// A join resource links the key to the row: many_to_many. The join rows are those
    /// `filter` selects (the join resource's read, as the actor sees it), each holding the
    /// key in `source` and the row's `attribute` in `destination`.
    Through {
        resource: &'a ResourceDef,
        filter: Option<&'a Filter>,
        source: &'a str,
        destination: &'a str,
        attribute: &'a str,
    },
}

impl CompiledQuery {
    /// Only explicit server predicates and sorts may require an unprojected value.
    pub fn needs_trusted_value(&self, field: &str) -> bool {
        let mut fields = Vec::new();
        if let Some(filter) = &self.filter {
            filter.collect_trusted_fields(&mut fields);
        }
        fields.contains(&field)
            || self
                .sort
                .iter()
                .any(|sort| !sort.projected && sort.field == field)
    }

    /// Validate opaque calculation contracts before execution, even for an empty table.
    pub fn validate_calculations(&self, resource: &ResourceDef) -> Result<()> {
        let mut names: Vec<&str> = self.calculations.iter().map(String::as_str).collect();
        names.extend(self.sort.iter().map(|sort| sort.field.as_str()));
        if let Some(filter) = &self.filter {
            filter.collect_fields(&mut names);
        }
        for name in names {
            if let Some(calc) = resource.calculation(name) {
                calc.validate_dependencies(resource)?;
            }
        }
        Ok(())
    }

    /// Whether the query reads `attribute` of `resource`: it's selected, or the primary
    /// key, or the query selects everything, or a Rust calculation declares it as an
    /// input.
    pub fn reads(&self, resource: &ResourceDef, attribute: &crate::resource::AttributeDef) -> bool {
        attribute.primary_key
            || self
                .select
                .as_ref()
                .is_none_or(|select| select.iter().any(|name| name == attribute.name))
            || self.custom_calculations(resource).any(|calc| {
                calc.dependency_fields()
                    .is_ok_and(|fields| fields.contains(&attribute.name))
                    || crate::field_policy_fields(resource).contains(&attribute.name)
            })
    }

    /// The calculations the query asks for that only Rust can compute: a SQL data layer
    /// computes them from the record it reads, after reading it.
    pub fn custom_calculations<'a>(
        &'a self,
        resource: &'a ResourceDef,
    ) -> impl Iterator<Item = &'a crate::expr::CalculationDef> + 'a {
        self.calculations
            .iter()
            .filter_map(|name| resource.calculation(name))
            .filter(|calc| calc.expr.is_custom())
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a valid Ash DataLayer",
    label = "not a DataLayer",
    note = "use Memory, Sqlite, Postgres, or implement DataLayer"
)]
/// Backend execution requires core-built plans, even when the app owns the backend.
///
/// ```compile_fail,E0308
/// async fn raw<D: ash_core::DataLayer>(data: &D, query: &ash_core::CompiledQuery) {
///     data.run_query(query).await;
/// }
/// ```
///
/// ```compile_fail,E0432
/// use ash_core::insert;
/// ```
pub trait DataLayer: Send + Sync {
    /// Write the prepared fields in the plan's bound tenant. Context tenancy selects
    /// the tenant namespace; attribute tenancy has already stamped the fields.
    fn create(&self, plan: &crate::CreatePlan<'_>)
    -> impl Future<Output = Result<FieldMap>> + Send;

    fn update(&self, plan: &crate::UpdatePlan<'_>)
    -> impl Future<Output = Result<FieldMap>> + Send;

    fn destroy(&self, plan: &crate::DestroyPlan<'_>) -> impl Future<Output = Result<()>> + Send;

    fn run_query(
        &self,
        plan: &crate::QueryPlan<'_>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send;

    /// Whether this data layer runs updates as one statement, checking conditions and
    /// raising their errors within it, as Ash's data layers that can `update_query` and
    /// `expr_error`. One that can't has updates read their record first.
    fn can_update_atomically(&self, _resource: &ResourceDef) -> bool {
        false
    }

    /// Whether this data layer raises a condition's error within an atomic statement, as
    /// Ash's data layers that can `expr_error`. One that updates atomically but can't (as
    /// AshSqlite) runs an update atomically only when nothing in it must raise.
    fn can_raise_atomically(&self, resource: &ResourceDef) -> bool {
        self.can_update_atomically(resource)
    }

    /// Runs `work` in one of this data layer's transactions, given the data layer that
    /// writes within it, as Ash runs an action in a transaction (`transaction?: true`)
    /// where its data layer can transact. Unsupported backends pass `None`; the core
    /// rejects multi-record actions before invoking their work.
    fn in_transaction<T, F, Fut>(&self, work: F) -> impl Future<Output = Result<T>> + Send
    where
        Self: Sized,
        F: FnOnce(Option<Self>) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        work(None)
    }

    /// Updates the records `query` selects as `update` says, in one statement: checks
    /// each record against the update's conditions, in order, failing with the first
    /// that holds, then sets the update's values, every one computed from the record as
    /// it was. Returns the updated records.
    fn update_atomic(
        &self,
        plan: &crate::AtomicUpdatePlan<'_>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "this data layer can't update {} atomically",
            plan.resource().name
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
        plan: &crate::AtomicDestroyPlan<'_>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "this data layer can't destroy {} atomically",
            plan.resource().name
        ))))
    }

    /// Whether this data layer runs a read of `resource` once for each of many keys in one
    /// statement, as AshPostgres loads a relationship with a lateral join, its rows related
    /// to the keys `by` says how: a join resource must be in the same store. One that
    /// can't has a relationship's limit and offset applied to each source's rows in
    /// memory, as Ash does for a data layer without lateral joins.
    fn can_run_query_per_key(&self, _resource: &ResourceDef, _by: &PerKey<'_>) -> bool {
        false
    }

    /// The rows of `resource` related to each of `keys`, read by `query` once per key:
    /// its filter and sort, and its limit and offset applying to each key's rows, as a
    /// lateral join applies them. `by` says how a row relates to a key. Returns each key's
    /// rows, in the order of `keys`, a key given twice getting its rows twice.
    fn run_query_per_key(
        &self,
        plan: &crate::PerKeyPlan<'_>,
    ) -> impl Future<Output = Result<Vec<Vec<FieldMap>>>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "this data layer can't read {} once per key",
            plan.resource().name
        ))))
    }

    /// How many records `query` would return, as Ash's data layers count with an
    /// aggregate query. A data layer that can count without reading the records (a SQL
    /// `COUNT(*)`) should; by default it reads them.
    fn count(&self, plan: &crate::CountPlan<'_>) -> impl Future<Output = Result<usize>> + Send {
        async move { Ok(self.run_query(plan.read()).await?.len()) }
    }

    /// A page of records, `page`, and how many records `count` matches, read at once, as
    /// Ash reads a page's count alongside it. By default they're read concurrently; a data
    /// layer that can send both down one connection together (Postgres pipelines them)
    /// should.
    fn run_query_with_count(
        &self,
        plan: &crate::QueryWithCountPlan<'_>,
    ) -> impl Future<Output = Result<(Vec<FieldMap>, usize)>> + Send {
        futures_util::future::try_join(self.run_query(plan.page()), self.count(plan.count()))
    }

    fn upsert(
        &self,
        plan: &crate::UpsertPlan<'_>,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        std::future::ready(Err(Error::Invalid(format!(
            "upsert is not supported for resource `{}` by this data layer",
            plan.resource().name
        ))))
    }

    fn bulk_create(
        &self,
        plan: &crate::BulkCreatePlan<'_>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        async move {
            let mut results = Vec::with_capacity(plan.rows().len());
            for row in plan.rows() {
                results.push(self.create(row).await?);
            }
            Ok(results)
        }
    }

    fn bulk_destroy(
        &self,
        plan: &crate::BulkDestroyPlan<'_>,
    ) -> impl Future<Output = Result<()>> + Send {
        async move {
            for row in plan.rows() {
                self.destroy(row).await?;
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
        plan: &crate::BulkUpdatePlan<'_>,
    ) -> impl Future<Output = Result<Vec<Result<FieldMap>>>> + Send {
        async move {
            let mut results = Vec::with_capacity(plan.rows().len());
            for row in plan.rows() {
                results.push(self.update(row).await);
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
        plan: &crate::CreatePlan<'_>,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(plan.resource())
    }

    fn update(
        &self,
        plan: &crate::UpdatePlan<'_>,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(plan.resource())
    }

    fn destroy(&self, plan: &crate::DestroyPlan<'_>) -> impl Future<Output = Result<()>> + Send {
        no_data_layer(plan.resource())
    }

    fn run_query(
        &self,
        plan: &crate::QueryPlan<'_>,
    ) -> impl Future<Output = Result<Vec<FieldMap>>> + Send {
        no_data_layer(plan.resource())
    }

    fn upsert(
        &self,
        plan: &crate::UpsertPlan<'_>,
    ) -> impl Future<Output = Result<FieldMap>> + Send {
        no_data_layer(plan.resource())
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

    /// Whether bulk actions write their batches in transactions here, as Ash asks
    /// `data_layer_can?(resource, :transact)` before it opens one for a batch. A data layer
    /// whose transactions are costly or only nominal says no, and bulk actions run
    /// without them.
    fn can_transact(&self) -> bool {
        true
    }
}

/// Nothing is stored, so a transaction just runs its work; bulk actions don't open one.
impl TransactionSupport for NoDataLayer {
    async fn transaction<F, Fut, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Self) -> Fut + Send,
        Fut: Future<Output = Result<T>> + Send,
        T: Send,
    {
        f(self).await
    }

    fn can_transact(&self) -> bool {
        false
    }
}
