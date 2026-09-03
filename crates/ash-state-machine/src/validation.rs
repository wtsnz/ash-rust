use ash_core::{CustomValidation, Error, Result, ValidationContext, Value};

use crate::error::InvalidTransition;

#[derive(Clone, Copy, Debug)]
pub struct TransitionValidation {
    pub state_attribute: &'static str,
    pub allowed_from: &'static [&'static str],
    pub target_state: &'static str,
    pub action: &'static str,
}

impl TransitionValidation {
    pub const fn new(
        state_attribute: &'static str,
        allowed_from: &'static [&'static str],
        target_state: &'static str,
        action: &'static str,
    ) -> Self {
        Self {
            state_attribute,
            allowed_from,
            target_state,
            action,
        }
    }
}

impl CustomValidation for TransitionValidation {
    fn validate(&self, ctx: &ValidationContext<'_>) -> Result<()> {
        let current_state = ctx
            .record
            .and_then(|r| r.get(self.state_attribute))
            .and_then(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .unwrap_or("");

        if !self.allowed_from.contains(&current_state) {
            return Err(Error::Extension(Box::new(InvalidTransition {
                current_state: current_state.to_string(),
                target_state: self.target_state.to_string(),
                action: self.action.to_string(),
            })));
        }

        Ok(())
    }
}
