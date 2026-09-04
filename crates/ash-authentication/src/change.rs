use ash_core::{ChangeContext, CustomChange, Error, Result, Value};

use crate::password::PasswordService;

/// Changeset lifecycle change that hashes a plaintext password from action arguments or fields
/// and assigns the resulting Argon2id hash to the target attribute.
#[derive(Clone, Copy, Debug)]
pub struct HashPasswordChange {
    password_argument: &'static str,
    confirmation_argument: Option<&'static str>,
    target_field: &'static str,
    min_length: usize,
}

impl HashPasswordChange {
    /// Create a new `HashPasswordChange`.
    pub const fn new(
        password_argument: &'static str,
        confirmation_argument: Option<&'static str>,
        target_field: &'static str,
    ) -> Self {
        Self {
            password_argument,
            confirmation_argument,
            target_field,
            min_length: 8,
        }
    }

    /// Customize the minimum password length requirement.
    pub const fn with_min_length(mut self, min: usize) -> Self {
        self.min_length = min;
        self
    }
}

impl CustomChange for HashPasswordChange {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        let password_val = ctx
            .arguments
            .get(self.password_argument)
            .or_else(|| ctx.fields.get(self.password_argument));

        let password = match password_val {
            Some(Value::String(s)) if !s.trim().is_empty() => s.as_str(),
            _ => {
                return Err(Error::Validation {
                    field: self.password_argument.to_string(),
                    message: "password is required".to_string(),
                });
            }
        };

        if let Some(conf_arg) = self.confirmation_argument {
            let conf_val = ctx
                .arguments
                .get(conf_arg)
                .or_else(|| ctx.fields.get(conf_arg));

            match conf_val {
                Some(Value::String(conf)) if conf == password => {}
                _ => {
                    return Err(Error::Validation {
                        field: conf_arg.to_string(),
                        message: "password confirmation does not match password".to_string(),
                    });
                }
            }
        }

        let password_service = PasswordService::new().with_min_length(self.min_length);
        let hashed = password_service
            .hash_password(password)
            .map_err(|e| Error::Validation {
                field: self.password_argument.to_string(),
                message: e.to_string(),
            })?;

        ctx.fields
            .insert(self.target_field.to_string(), Value::String(hashed));

        // Clean up plaintext password if it was somehow placed in fields
        ctx.fields.remove(self.password_argument);
        if let Some(conf_arg) = self.confirmation_argument {
            ctx.fields.remove(conf_arg);
        }

        Ok(())
    }
}
