use ash_core::ResourceExtension;
use std::any::Any;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransitionDef {
    pub action: &'static str,
    pub from: &'static [&'static str],
    pub to: &'static str,
}

impl TransitionDef {
    pub const fn new(action: &'static str, from: &'static [&'static str], to: &'static str) -> Self {
        Self { action, from, to }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateMachineDef {
    pub state_attribute: &'static str,
    pub initial_states: &'static [&'static str],
    pub default_initial_state: Option<&'static str>,
    pub transitions: &'static [TransitionDef],
}

impl StateMachineDef {
    pub const fn new(
        state_attribute: &'static str,
        initial_states: &'static [&'static str],
        default_initial_state: Option<&'static str>,
        transitions: &'static [TransitionDef],
    ) -> Self {
        Self {
            state_attribute,
            initial_states,
            default_initial_state,
            transitions,
        }
    }

    pub fn transitions_for_action<'a>(
        &'a self,
        action_name: &'a str,
    ) -> impl Iterator<Item = &'a TransitionDef> {
        self.transitions.iter().filter(move |t| t.action == action_name)
    }

    pub fn find_transition(&self, current_state: &str, action_name: &str) -> Option<&TransitionDef> {
        self.transitions
            .iter()
            .find(|t| t.action == action_name && t.from.contains(&current_state))
    }

    pub fn can_transition(&self, current_state: &str, action_name: &str) -> bool {
        self.find_transition(current_state, action_name).is_some()
    }

    pub fn possible_next_states(&self, current_state: &str) -> Vec<&'static str> {
        self.transitions
            .iter()
            .filter(|t| t.from.contains(&current_state))
            .map(|t| t.to)
            .collect()
    }
}

impl ResourceExtension for StateMachineDef {
    fn name(&self) -> &'static str {
        "StateMachine"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
