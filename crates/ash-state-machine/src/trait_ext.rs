use ash_core::Resource;
use crate::def::StateMachineDef;

pub trait HasStateMachine: Resource {
    const STATE_MACHINE: StateMachineDef;

    fn current_state(&self) -> &str;

    fn can_transition(&self, action: &str) -> bool {
        Self::STATE_MACHINE.can_transition(self.current_state(), action)
    }

    fn possible_next_states(&self) -> Vec<&'static str> {
        Self::STATE_MACHINE.possible_next_states(self.current_state())
    }
}
