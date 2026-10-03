use ash_core::{
    Atomic, AtomicCondition, AtomicContext, AtomicExpr, CustomValidation, Error, Result,
    ValidationContext, Value,
};

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
            return Err(self.invalid(current_state));
        }

        Ok(())
    }

    /// Fails when the record isn't in a state the transition is from, as AshStateMachine's
    /// atomic `transition_state` does: the condition reads the state as stored, and the
    /// error says which it was.
    fn atomic(&self, _ctx: &AtomicContext<'_>) -> Atomic {
        let state = || Box::new(AtomicExpr::field(self.state_attribute));
        let allowed = self.allowed_from.iter().map(|s| Value::String((*s).to_string())).collect();
        let this = *self;
        Atomic::conditions(vec![AtomicCondition::new(
            AtomicExpr::Or(vec![AtomicExpr::IsNil(state()), AtomicExpr::Not(Box::new(AtomicExpr::In(state(), allowed)))]),
            vec![self.state_attribute.to_string()],
            move |row| {
                let current = row.get(this.state_attribute).and_then(Value::as_str).unwrap_or("");
                this.invalid(current)
            },
        )])
    }
}

impl TransitionValidation {
    fn invalid(&self, current_state: &str) -> Error {
        Error::Extension(Box::new(InvalidTransition {
            current_state: current_state.to_string(),
            target_state: self.target_state.to_string(),
            action: self.action.to_string(),
        }))
    }
}
