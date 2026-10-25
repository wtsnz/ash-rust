//! Immutable backend requests. Only core can turn scoped execution into a plan.

use crate::{
    Actor, AtomicCondition, AtomicUpdate, CompiledQuery, Context, FieldMap, PerKey, ResourceDef,
    Value,
};
use std::sync::Arc;

/// The bound identity retained by an executable plan. It grants no rebinding authority.
///
/// ```compile_fail,E0624
/// use ash_core::PlanScope;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = PlanScope::new(ctx);
/// }
/// ```
pub struct PlanScope(Arc<crate::context::Scope>);
impl PlanScope {
    pub(crate) fn new<D>(ctx: &Context<D>) -> Self {
        Self(Arc::clone(&ctx.scope))
    }
    pub fn actor(&self) -> Option<&Actor> {
        self.0.actor()
    }
    pub fn tenant(&self) -> Option<&str> {
        self.0.tenant()
    }
}

/// A fully planned read. `CompiledQuery` alone is only a compiler input.
///
/// ```compile_fail,E0624
/// use ash_core::QueryPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = QueryPlan::new(ctx, resource, &ash_core::CompiledQuery::default());
/// }
/// ```
pub struct QueryPlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    query: &'a CompiledQuery,
}
impl<'a> QueryPlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
    ) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            query,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn query(&self) -> &CompiledQuery {
        self.query
    }
}

/// A count over a scoped query.
///
/// ```compile_fail,E0624
/// use ash_core::CountPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = CountPlan::new(ctx, resource, &ash_core::CompiledQuery::default());
/// }
/// ```
pub struct CountPlan<'a> {
    read: QueryPlan<'a>,
}
impl<'a> CountPlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
    ) -> Self {
        Self {
            read: QueryPlan::new(ctx, resource, query),
        }
    }
    pub fn read(&self) -> &QueryPlan<'a> {
        &self.read
    }
    pub fn scope(&self) -> &PlanScope {
        self.read.scope()
    }
    pub fn resource(&self) -> &ResourceDef {
        self.read.resource()
    }
    pub fn query(&self) -> &CompiledQuery {
        self.read.query()
    }
}

/// A page and its count, planned under the same bound scope.
///
/// ```compile_fail,E0624
/// use ash_core::QueryWithCountPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let query = ash_core::CompiledQuery::default(); let _ = QueryWithCountPlan::new(ctx, resource, &query, &query);
/// }
/// ```
pub struct QueryWithCountPlan<'a> {
    page: QueryPlan<'a>,
    count: CountPlan<'a>,
}
impl<'a> QueryWithCountPlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        page: &'a CompiledQuery,
        count: &'a CompiledQuery,
    ) -> Self {
        Self {
            page: QueryPlan::new(ctx, resource, page),
            count: CountPlan::new(ctx, resource, count),
        }
    }
    pub fn scope(&self) -> &PlanScope {
        self.page.scope()
    }
    pub fn resource(&self) -> &ResourceDef {
        self.page.resource()
    }
    pub fn page(&self) -> &QueryPlan<'a> {
        &self.page
    }
    pub fn count(&self) -> &CountPlan<'a> {
        &self.count
    }
}

/// A relationship read with scoped destination and join predicates.
///
/// ```compile_fail,E0624
/// use ash_core::PerKeyPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = PerKeyPlan::new(ctx, resource, &ash_core::CompiledQuery::default(), ash_core::PerKey::Attribute("id"), &[]);
/// }
/// ```
pub struct PerKeyPlan<'a> {
    read: QueryPlan<'a>,
    by: PerKey<'a>,
    keys: &'a [Value],
}
impl<'a> PerKeyPlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
        by: PerKey<'a>,
        keys: &'a [Value],
    ) -> Self {
        Self {
            read: QueryPlan::new(ctx, resource, query),
            by,
            keys,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        self.read.scope()
    }
    pub fn resource(&self) -> &ResourceDef {
        self.read.resource()
    }
    pub fn query(&self) -> &CompiledQuery {
        self.read.query()
    }
    pub fn by(&self) -> &PerKey<'a> {
        &self.by
    }
    pub fn keys(&self) -> &[Value] {
        self.keys
    }
}

/// A prepared and authorized create of one record.
///
/// ```compile_fail,E0624
/// use ash_core::CreatePlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = CreatePlan::new(ctx, resource, ash_core::Value::Null, ash_core::FieldMap::new());
/// }
/// ```
pub struct CreatePlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    id: Value,
    fields: FieldMap,
}
impl<'a> CreatePlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        id: Value,
        mut fields: FieldMap,
    ) -> Self {
        crate::CredentialVersionRule::initialize(resource, &mut fields);
        Self {
            scope: PlanScope::new(ctx),
            resource,
            id,
            fields,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn id(&self) -> &Value {
        &self.id
    }
    pub fn fields(&self) -> &FieldMap {
        &self.fields
    }
}

/// A batch of individually prepared and authorized creates.
///
/// ```compile_fail,E0624
/// use ash_core::BulkCreatePlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = BulkCreatePlan::new(ctx, resource, Vec::new());
/// }
/// ```
pub struct BulkCreatePlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    rows: Vec<CreatePlan<'a>>,
}
impl<'a> BulkCreatePlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        rows: Vec<(Value, FieldMap)>,
    ) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            rows: rows
                .into_iter()
                .map(|(id, fields)| CreatePlan::new(ctx, resource, id, fields))
                .collect(),
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn rows(&self) -> &[CreatePlan<'a>] {
        &self.rows
    }
}

/// A prepared and authorized update of one record.
///
/// ```compile_fail,E0624
/// use ash_core::UpdatePlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = UpdatePlan::new(ctx, resource, ash_core::Value::Null, ash_core::FieldMap::new());
/// }
/// ```
pub struct UpdatePlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    id: Value,
    fields: FieldMap,
}
impl<'a> UpdatePlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        id: Value,
        fields: FieldMap,
    ) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            id,
            fields,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn id(&self) -> &Value {
        &self.id
    }
    pub fn fields(&self) -> &FieldMap {
        &self.fields
    }
}

/// A batch of individually prepared and authorized updates.
///
/// ```compile_fail,E0624
/// use ash_core::BulkUpdatePlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = BulkUpdatePlan::new(ctx, resource, Vec::new());
/// }
/// ```
pub struct BulkUpdatePlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    rows: Vec<UpdatePlan<'a>>,
}
impl<'a> BulkUpdatePlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        rows: Vec<(Value, FieldMap)>,
    ) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            rows: rows
                .into_iter()
                .map(|(id, fields)| UpdatePlan::new(ctx, resource, id, fields))
                .collect(),
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn rows(&self) -> &[UpdatePlan<'a>] {
        &self.rows
    }
}

/// An authorized deletion of one record.
///
/// ```compile_fail,E0624
/// use ash_core::DestroyPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = DestroyPlan::new(ctx, resource, ash_core::Value::Null);
/// }
/// ```
pub struct DestroyPlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    id: Value,
}
impl<'a> DestroyPlan<'a> {
    pub(crate) fn new<D>(ctx: &Context<D>, resource: &'a ResourceDef, id: Value) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            id,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn id(&self) -> &Value {
        &self.id
    }
}

/// A batch of individually authorized deletions.
///
/// ```compile_fail,E0624
/// use ash_core::BulkDestroyPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = BulkDestroyPlan::new(ctx, resource, &[]);
/// }
/// ```
pub struct BulkDestroyPlan<'a> {
    scope: PlanScope,
    resource: &'a ResourceDef,
    rows: Vec<DestroyPlan<'a>>,
}
impl<'a> BulkDestroyPlan<'a> {
    pub(crate) fn new<D>(ctx: &Context<D>, resource: &'a ResourceDef, ids: &[Value]) -> Self {
        Self {
            scope: PlanScope::new(ctx),
            resource,
            rows: ids
                .iter()
                .map(|id| DestroyPlan::new(ctx, resource, id.clone()))
                .collect(),
        }
    }
    pub fn scope(&self) -> &PlanScope {
        &self.scope
    }
    pub fn resource(&self) -> &ResourceDef {
        self.resource
    }
    pub fn tenant(&self) -> Option<&str> {
        self.scope.tenant()
    }
    pub fn rows(&self) -> &[DestroyPlan<'a>] {
        &self.rows
    }
}

/// An atomic update with its scoped selection, policy and validation conditions.
///
/// ```compile_fail,E0624
/// use ash_core::AtomicUpdatePlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = AtomicUpdatePlan::new(ctx, resource, &ash_core::CompiledQuery::default(), &ash_core::AtomicUpdate::default());
/// }
/// ```
pub struct AtomicUpdatePlan<'a> {
    read: QueryPlan<'a>,
    update: AtomicUpdate,
}
impl<'a> AtomicUpdatePlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
        update: &'a AtomicUpdate,
    ) -> Self {
        let mut update = update.clone();
        crate::CredentialVersionRule::apply(resource, &mut update);
        Self {
            read: QueryPlan::new(ctx, resource, query),
            update,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        self.read.scope()
    }
    pub fn resource(&self) -> &ResourceDef {
        self.read.resource()
    }
    pub fn query(&self) -> &CompiledQuery {
        self.read.query()
    }
    pub fn update(&self) -> &AtomicUpdate {
        &self.update
    }
}

/// An atomic deletion with its scoped selection, policy and validation conditions.
///
/// ```compile_fail,E0624
/// use ash_core::AtomicDestroyPlan;
/// fn forge(ctx: &ash_core::Context<()>, resource: &ash_core::ResourceDef) {
///     let _ = AtomicDestroyPlan::new(ctx, resource, &ash_core::CompiledQuery::default(), &[]);
/// }
/// ```
pub struct AtomicDestroyPlan<'a> {
    read: QueryPlan<'a>,
    conditions: &'a [AtomicCondition],
}
impl<'a> AtomicDestroyPlan<'a> {
    pub(crate) fn new<D>(
        ctx: &Context<D>,
        resource: &'a ResourceDef,
        query: &'a CompiledQuery,
        conditions: &'a [AtomicCondition],
    ) -> Self {
        Self {
            read: QueryPlan::new(ctx, resource, query),
            conditions,
        }
    }
    pub fn scope(&self) -> &PlanScope {
        self.read.scope()
    }
    pub fn resource(&self) -> &ResourceDef {
        self.read.resource()
    }
    pub fn query(&self) -> &CompiledQuery {
        self.read.query()
    }
    pub fn conditions(&self) -> &[AtomicCondition] {
        self.conditions
    }
}
