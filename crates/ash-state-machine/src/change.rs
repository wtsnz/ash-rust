use ash_core::{Atomic, AtomicContext, AtomicExpr, ChangeContext, CustomChange, Result, Value};

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

    /// Sets the target state, as AshStateMachine's `transition_state` does atomically. The
    /// transition's validation checks the state the record is in.
    fn atomic(&self, _ctx: &AtomicContext<'_>) -> Atomic {
        Atomic::Atomic {
            set: vec![(self.state_attribute.to_string(), AtomicExpr::value(self.target_state))],
            conditions: Vec::new(),
        }
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

    fn atomic(&self, ctx: &AtomicContext<'_>) -> Atomic {
        Atomic::Atomic {
            set: vec![(
                self.state_attribute.to_string(),
                AtomicExpr::Coalesce(vec![ctx.value_of(self.state_attribute), AtomicExpr::value(self.default_state)]),
            )],
            conditions: Vec::new(),
        }
    }
}
