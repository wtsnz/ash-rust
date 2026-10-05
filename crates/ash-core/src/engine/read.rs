//! What a read sees. Typed queries, relationship loads, and the GraphQL resolvers all
//! build their reads here, so they agree on preparations, policies and tenancy.

use crate::action::{ActionDef, PreparationDef};
use crate::actor::Actor;
use crate::data_layer::{CompiledQuery, Sort};
use crate::error::Result;
use crate::filter::Filter;
use crate::pipeline::apply_tenant_scope;
use crate::policy::compile_read_filter;
use crate::resource::ResourceDef;
use crate::value::FieldMap;

/// The caller's `query` as a read through `action` runs it:
///
/// - the action's filter preparations, its sort after the caller's, and its limit and
///   offset where the caller gave none;
/// - an equality filter for each argument that names an attribute;
/// - the actor's read policies, or `Forbidden` when none covers the action;
/// - the tenant scope for `query.tenant`, or `TenantRequired` when a tenant-scoped
///   resource is read without one.
pub fn scope_read(
    resource: &ResourceDef,
    action: &ActionDef,
    actor: Option<&Actor>,
    arguments: &FieldMap,
    mut query: CompiledQuery,
) -> Result<CompiledQuery> {
    let mut filters = Vec::new();
    let mut prepared_sort = Vec::new();
    for preparation in action.preparations {
        match *preparation {
            PreparationDef::Filter(build) => filters.push(build()),
            PreparationDef::FilterWithArgs(build) => filters.push(build(arguments)),
            PreparationDef::Sort { field, descending } => prepared_sort.push(Sort {
                field: field.to_string(),
                descending,
                ..Default::default()
            }),
            PreparationDef::Limit(limit) => {
                query.limit.get_or_insert(limit);
            }
            PreparationDef::Offset(offset) => {
                query.offset.get_or_insert(offset);
            }
            PreparationDef::AfterAction(_) => {}
        }
    }
    // The query's sort, then the action's on fields it doesn't sort by, as Ash appends
    // what a read's preparations sort by to a query that already sorts (as a code
    // interface's or AshGraphql's does).
    for sort in prepared_sort {
        if !query.sort.iter().any(|given| given.field == sort.field) {
            query.sort.push(sort);
        }
    }
    for (name, value) in arguments {
        if resource.attribute(name).is_some() {
            filters.push(Filter::eq(name.as_str(), value.clone()));
        }
    }
    filters.extend(query.filter.take());
    // A tenant a resource needs is asked for before its policies are, as Ash asks: an
    // anonymous read with no tenant is `TenantRequired`, not `Forbidden`.
    apply_tenant_scope(resource, None, query.tenant.clone())?;
    filters.extend(compile_read_filter(resource, action, actor)?);
    let filter = match Filter::and(filters) {
        Filter::True => None,
        filter => Some(filter),
    };
    let (filter, tenant) = apply_tenant_scope(resource, filter, query.tenant.take())?;
    query.filter = filter;
    query.tenant = tenant;
    query.actor = actor.cloned();
    Ok(query)
}

/// Runs `action`'s `after_action` preparations on the records a read through it found,
/// as Ash runs a read's after-action hooks: every read through the action calls it.
pub fn after_read(action: &ActionDef, arguments: &FieldMap, records: &mut [FieldMap]) -> Result<()> {
    for preparation in action.preparations {
        if let PreparationDef::AfterAction(run) = preparation {
            run(arguments, records)?;
        }
    }
    Ok(())
}

/// Whether `actor` in `tenant` would see `record` through the resource's default read.
/// Subscriptions use it to decide who hears about a change.
pub fn record_visible(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    tenant: Option<&str>,
    record: &FieldMap,
) -> bool {
    let read = resource.default_read();
    let query = CompiledQuery {
        tenant: tenant.map(str::to_string),
        ..CompiledQuery::default()
    };
    match scope_read(resource, read, actor, &FieldMap::new(), query) {
        Ok(scoped) => scoped
            .filter
            .is_none_or(|filter| filter.matches_on(resource, record)),
        Err(_) => false,
    }
}
