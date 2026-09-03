use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidTransition {
    pub current_state: String,
    pub target_state: String,
    pub action: String,
}

impl fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot transition from `{}` to `{}` on action `{}`",
            self.current_state, self.target_state, self.action
        )
    }
}

impl std::error::Error for InvalidTransition {}
