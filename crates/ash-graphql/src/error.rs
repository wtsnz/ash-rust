use ash_core::Error as AshError;
use async_graphql::Value as GqlValue;
use async_graphql::dynamic::*;

use crate::names::camel;

/// The GraphQL type a mutation's errors are, as AshGraphql names it.
pub const MUTATION_ERROR: &str = "MutationError";

/// An error a mutation reports, rather than raising: a validation, authorization or
/// conflict failure, with Ash's error code (`invalid_attribute`, `required`,
/// `not_found`, …) and the field it's about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserError {
    pub message: String,
    pub field: Option<String>,
    pub code: String,
}

impl UserError {
    /// The error Ash would report for `err`.
    pub fn from_ash_error(err: &AshError) -> Self {
        let (message, field, code) = match err {
            AshError::Forbidden => (
                "forbidden".to_string(),
                None,
                "forbidden",
            ),
            AshError::NotFound => ("could not be found".to_string(), Some("id".to_string()), "not_found"),
            // A validation's message with its vars filled in, as AshGraphql writes it.
            AshError::Validation { field, .. } => (err.message(), Some(field.clone()), "invalid_attribute"),
            AshError::Constraint { field, message } => (message.clone(), Some(field.clone()), "invalid_attribute"),
            AshError::Missing { field } => ("is required".to_string(), Some(field.clone()), "required"),
            AshError::StaleRecord { resource, id } => (
                format!("{resource} {id} was changed by someone else"),
                Some("version".to_string()),
                "stale_record",
            ),
            AshError::IdentityConflict {
                fields, message, ..
            } => (
                message.clone(),
                fields.first().cloned(),
                "invalid_attribute",
            ),
            other => (other.to_string(), None, "unknown"),
        };
        Self {
            message,
            field,
            code: code.to_string(),
        }
    }
}

/// Registers `MutationError { message, shortMessage, vars, code, fields, path }`.
pub fn register_user_error(builder: SchemaBuilder) -> SchemaBuilder {
    let error = |ctx: &ResolverContext<'_>| ctx.parent_value.downcast_ref::<UserError>().cloned();
    let text = |value: String| Some(FieldValue::value(GqlValue::String(value)));
    builder.register(
        Object::new(MUTATION_ERROR)
            .field(Field::new("message", TypeRef::named(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move { Ok(error(&ctx).and_then(|e| text(e.message))) })
            }))
            .field(Field::new("shortMessage", TypeRef::named(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move { Ok(error(&ctx).and_then(|e| text(e.message))) })
            }))
            .field(Field::new("vars", TypeRef::named("Json"), move |_ctx| {
                FieldFuture::new(async move {
                    Ok(Some(FieldValue::value(GqlValue::Object(Default::default()))))
                })
            }))
            .field(Field::new("code", TypeRef::named(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move { Ok(error(&ctx).and_then(|e| text(e.code))) })
            }))
            .field(Field::new("fields", TypeRef::named_nn_list(TypeRef::STRING), move |ctx| {
                FieldFuture::new(async move {
                    let fields = error(&ctx).and_then(|e| e.field).map(|f| camel(&f));
                    Ok(Some(FieldValue::list(
                        fields.into_iter().map(|f| FieldValue::value(GqlValue::String(f))),
                    )))
                })
            }))
            .field(Field::new("path", TypeRef::named_nn_list(TypeRef::STRING), move |_ctx| {
                FieldFuture::new(async move { Ok(None::<FieldValue>) })
            })),
    )
}

/// Gives a resolver's error the path of the field it failed at, as Absinthe reports
/// it: async-graphql's dynamic schemas leave it without one.
pub(crate) struct ErrorPaths;

impl async_graphql::extensions::ExtensionFactory for ErrorPaths {
    fn create(&self) -> std::sync::Arc<dyn async_graphql::extensions::Extension> {
        std::sync::Arc::new(ErrorPaths)
    }
}

#[async_graphql::async_trait::async_trait]
impl async_graphql::extensions::Extension for ErrorPaths {
    async fn resolve(
        &self,
        ctx: &async_graphql::extensions::ExtensionContext<'_>,
        info: async_graphql::extensions::ResolveInfo<'_>,
        next: async_graphql::extensions::NextResolve<'_>,
    ) -> async_graphql::ServerResult<Option<GqlValue>> {
        let path_node = info.path_node;
        next.run(ctx, info).await.map_err(|mut error| {
            if error.path.is_empty() {
                let mut node = Some(path_node);
                while let Some(current) = node {
                    error.path.push(match current.segment {
                        async_graphql::QueryPathSegment::Name(name) => async_graphql::PathSegment::Field(name.to_string()),
                        async_graphql::QueryPathSegment::Index(index) => async_graphql::PathSegment::Index(index),
                    });
                    node = current.parent;
                }
                error.path.reverse();
            }
            error
        })
    }
}

/// The error a generic action's failure raises, as AshGraphql raises it at the top
/// level: its message, with Ash's code, the field it's about and its vars.
pub(crate) fn raised(err: &AshError) -> async_graphql::Error {
    use async_graphql::ErrorExtensions;
    let error = UserError::from_ash_error(err);
    async_graphql::Error::new(error.message.clone()).extend_with(|_, extensions| {
        extensions.set("code", error.code.as_str());
        extensions.set("short_message", error.message.as_str());
        extensions.set("vars", GqlValue::Object(Default::default()));
        extensions.set(
            "fields",
            GqlValue::List(error.field.iter().map(|field| GqlValue::String(camel(field))).collect()),
        );
    })
}
