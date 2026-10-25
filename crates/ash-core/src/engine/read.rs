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
    query: CompiledQuery,
) -> Result<CompiledQuery> {
    scope_discovery(resource, action, actor, arguments, query, false)
}

/// Hard cascade discovery includes archived records, retaining all other read rules.
pub(crate) fn scope_discovery(
    resource: &ResourceDef,
    action: &ActionDef,
    actor: Option<&Actor>,
    arguments: &FieldMap,
    mut query: CompiledQuery,
    hard: bool,
) -> Result<CompiledQuery> {
    let soft_delete = hard
        .then(|| {
            resource
                .extensions
                .iter()
                .find_map(|extension| extension.soft_delete_attribute())
        })
        .flatten();
    let mut filters = Vec::new();
    let mut prepared_sort = Vec::new();
    for preparation in action.preparations {
        match *preparation {
            PreparationDef::Filter(build) => {
                let filter = build().into_filter();
                if !matches!(&filter, Filter::IsNil(field) if Some(field.as_str()) == soft_delete) {
                    filters.push(filter);
                }
            }
            PreparationDef::FilterWithArgs(build) => {
                let filter = build(arguments).into_filter();
                filters.push(filter);
            }
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
    filters.push(crate::read_plan::argument_filter(resource, arguments));
    filters.extend(query.filter.take());
    filters.extend(compile_read_filter(resource, action, actor)?);
    let planned = crate::plan_filter(
        resource,
        actor,
        query.tenant.as_deref(),
        crate::TrustedExpr::Filter(Filter::and(filters)),
    )?;
    let filter = match planned {
        Filter::True => None,
        filter => Some(filter),
    };
    let (filter, tenant) = apply_tenant_scope(resource, filter, query.tenant.take())?;
    query.filter = filter;
    query.tenant = tenant;
    if let Some(distinct) = &mut query.distinct_on
        && distinct.sort.is_empty()
    {
        distinct.sort = crate::keyset_sort(resource, query.sort.clone());
    }
    query.actor = actor.cloned();
    Ok(query)
}

/// Runs `action`'s `after_action` preparations on the records a read through it found,
/// as Ash runs a read's after-action hooks: every read through the action calls it.
pub fn after_read(
    action: &ActionDef,
    arguments: &FieldMap,
    records: &mut [FieldMap],
) -> Result<()> {
    let mut raw: Vec<crate::RawRecord> = records
        .iter_mut()
        .map(|fields| std::mem::take(fields).into())
        .collect();
    let result = (|| {
        for preparation in action.preparations {
            if let PreparationDef::AfterAction(run) = preparation {
                run(arguments, &mut raw)?;
            }
        }
        Ok(())
    })();
    for (record, raw) in records.iter_mut().zip(raw) {
        *record = raw.into_fields();
    }
    result?;
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
        Ok(scoped) => scoped.filter.is_none_or(|filter| {
            filter.snapshot_match(resource, record) == crate::FilterMatch::Match
        }),
        Err(_) => false,
    }
}
