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
            AshError::Validation { field, message } | AshError::Constraint { field, message } => {
                (message.clone(), Some(field.clone()), "invalid_attribute")
            }
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
