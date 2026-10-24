//! Field policies, applied as Ash applies them: they hide attribute, calculation and
//! aggregate values from the requester, not relationships. A relationship loads through
//! its real keys, whoever reads it, and its destination's read policies decide what's seen.

use async_graphql::{ErrorExtensions, Value as GqlValue};

/// Reports the field `ctx` resolves as hidden by a field policy, as AshGraphql's
/// `forbidden_field` error at its path; the field resolves to null. (Reported here, not
/// returned: async-graphql's dynamic schemas drop a field whose resolver fails, and
/// leave its error without a path.)
pub(crate) fn report_forbidden(ctx: &async_graphql::dynamic::ResolverContext<'_>) {
    let error = async_graphql::Error::new("forbidden field").extend_with(|_, extensions| {
        extensions.set("code", "forbidden_field");
        extensions.set("short_message", "forbidden field");
        extensions.set("vars", GqlValue::Object(Default::default()));
        extensions.set("fields", GqlValue::List(Vec::new()));
    });
    let error = ctx
        .ctx
        .set_error_path(error.into_server_error(ctx.ctx.item.pos));
    ctx.ctx.add_error(error);
}
