use ash_core::{Actor, FieldMap, Filter, ResourceDef, compile_read_filter};

fn and_filters(left: Option<Filter>, right: Option<Filter>) -> Option<Filter> {
    match (left, right) {
        (None, None) => None,
        (Some(filter), None) | (None, Some(filter)) => Some(filter),
        (Some(left), Some(right)) => Some(Filter::and([left, right])),
    }
}

/// AND the resource's primary read policy onto a user filter.
///
/// `Forbidden` (no applicable read policy) becomes a filter that matches nothing
/// so GraphQL loaders and subscriptions fail closed.
pub fn scoped_read_filter(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    user_filter: Option<Filter>,
) -> Option<Filter> {
    let Some(read) = resource.primary_read() else {
        return user_filter;
    };
    match compile_read_filter(resource, read, actor) {
        Ok(policy) => and_filters(user_filter, policy),
        Err(_) => Some(Filter::False),
    }
}

/// Whether this actor would see `record` on a primary read.
pub fn record_visible_for_read(
    resource: &ResourceDef,
    actor: Option<&Actor>,
    record: &FieldMap,
) -> bool {
    match scoped_read_filter(resource, actor, None) {
        None => true,
        Some(filter) => filter.matches(record),
    }
}
