use ash_core::{ChangeContext, CustomChange, Result, Value};

#[derive(Clone, Copy, Debug)]
pub struct TransitionChange {
    pub state_attribute: &'static str,
    pub target_state: &'static str,
}

impl TransitionChange {
    pub const fn new(state_attribute: &'static str, target_state: &'static str) -> Self {
        Self {
            state_attribute,
            target_state,
        }
    }
}

impl CustomChange for TransitionChange {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        ctx.fields.insert(
            self.state_attribute.to_string(),
            Value::String(self.target_state.to_string()),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DefaultStateChange {
    pub state_attribute: &'static str,
    pub default_state: &'static str,
}

impl DefaultStateChange {
    pub const fn new(state_attribute: &'static str, default_state: &'static str) -> Self {
        Self {
            state_attribute,
            default_state,
        }
    }
}

impl CustomChange for DefaultStateChange {
    fn apply(&self, ctx: &mut ChangeContext<'_>) -> Result<()> {
        if !ctx.fields.contains_key(self.state_attribute)
            || ctx.fields.get(self.state_attribute) == Some(&Value::Null)
        {
            ctx.fields.insert(
                self.state_attribute.to_string(),
                Value::String(self.default_state.to_string()),
            );
        }
        Ok(())
    }
}
