use ash_core::Error as AshError;
use async_graphql::dynamic::*;
use async_graphql::Value as GqlValue;

/// Structured user error payload representing business validation, authorization, or conflict failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserError {
    pub message: String,
    pub field: Option<String>,
    pub code: String,
}

impl UserError {
    /// Converts an [`ash_core::Error`] into a structured [`UserError`].
    pub fn from_ash_error(err: &AshError) -> Self {
        match err {
            AshError::Forbidden => Self {
                message: "Forbidden: insufficient permissions to perform action".into(),
                field: None,
                code: "FORBIDDEN".into(),
            },
            AshError::NotFound => Self {
                message: "Record not found".into(),
                field: Some("id".into()),
                code: "NOT_FOUND".into(),
            },
            AshError::Validation { field, message }
            | AshError::Constraint { field, message } => Self {
                message: message.clone(),
                field: Some(field.clone()),
                code: "VALIDATION_FAILED".into(),
            },
            AshError::Missing { field } => Self {
                message: format!("Required field `{field}` is missing"),
                field: Some(field.clone()),
                code: "REQUIRED_FIELD_MISSING".into(),
            },
            AshError::StaleRecord { resource, id } => Self {
                message: format!(
                    "Resource `{resource}` with id `{id}` was modified concurrently"
                ),
                field: Some("version".into()),
                code: "STALE_RECORD".into(),
            },
            AshError::IdentityConflict {
                identity, message, ..
            } => Self {
                message: message.clone(),
                field: Some(identity.to_string()),
                code: "IDENTITY_CONFLICT".into(),
            },
            other => Self {
                message: other.to_string(),
                field: None,
                code: "INTERNAL_ERROR".into(),
            },
        }
    }
}

/// Registers the shared `UserError` GraphQL object type.
pub fn register_user_error(builder: SchemaBuilder) -> SchemaBuilder {
    let user_error_obj = Object::new("UserError")
        .field(Field::new(
            "message",
            TypeRef::named_nn(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(err) = ctx.parent_value.downcast_ref::<UserError>() {
                        Ok(Some(FieldValue::value(GqlValue::String(
                            err.message.clone(),
                        ))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::String(String::new()))))
                    }
                })
            },
        ))
        .field(Field::new(
            "field",
            TypeRef::named(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(err) = ctx.parent_value.downcast_ref::<UserError>() {
                        match &err.field {
                            Some(f) => Ok(Some(FieldValue::value(GqlValue::String(f.clone())))),
                            None => Ok(None),
                        }
                    } else {
                        Ok(None)
                    }
                })
            },
        ))
        .field(Field::new(
            "code",
            TypeRef::named_nn(TypeRef::STRING),
            |ctx| {
                FieldFuture::new(async move {
                    if let Some(err) = ctx.parent_value.downcast_ref::<UserError>() {
                        Ok(Some(FieldValue::value(GqlValue::String(err.code.clone()))))
                    } else {
                        Ok(Some(FieldValue::value(GqlValue::String(
                            "UNKNOWN".into(),
                        ))))
                    }
                })
            },
        ));

    builder.register(user_error_obj)
}
