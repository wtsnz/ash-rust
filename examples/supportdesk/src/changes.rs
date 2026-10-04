//! Changes the domain's actions share.

use ash_core::{Atomic, AtomicContext, AtomicExpr, ChangeContext, CustomChange, Result, Value};

/// Adds one to an attribute in the statement that updates the record, as Ash's
/// `atomic_update(attribute, expr(attribute + 1))` does, so concurrent updates each count.
pub struct Increment(pub &'static str);

impl CustomChange for Increment {
    /// The update always runs atomically, as the action requires; there is nothing to set
    /// before it, since the stored value isn't known until the statement runs.
    fn apply(&self, _ctx: &mut ChangeContext<'_>) -> Result<()> {
        Ok(())
    }

    fn atomic(&self, _ctx: &AtomicContext<'_>) -> Atomic {
        Atomic::Atomic {
            set: vec![(
                self.0.to_string(),
                AtomicExpr::Add(
                    Box::new(AtomicExpr::Field(self.0.to_string())),
                    Box::new(AtomicExpr::Value(Value::Int(1))),
                ),
            )],
            conditions: Vec::new(),
        }
    }
}

pub static COUNT_VIEW: Increment = Increment("view_count");
pub static COUNT_REOPEN: Increment = Increment("reopen_count");
