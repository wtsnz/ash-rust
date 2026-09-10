use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::representative::Representative;
use ash_core::{Actor, Result, resource};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            other => Err(ash_core::Error::Invalid(format!("unknown status {other}"))),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub word_count: usize,
    pub urgent: bool,
}

resource! {
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        subject: String,
        status: Status [atom: "open,closed"],
        opener_id: Uuid,
        representative_id: Option<Uuid>,
    }

    relationships {
        belongs_to representative: Option<Representative>,
    }

    calculations {
        subject_length: Option<i64> = string_length(subject),
    }

    actions {
        create open {
            accept {
                subject: String,
            }
            validate present(subject);
            validate string_length(subject, min = 2);
            change set(status = "open");
            change relate_actor(opener_id);
        }

        read read {
            primary
        }

        update assign {
            accept {
                representative_id: Uuid,
            }
        }

        update close {
            change set(status = "closed");
        }

        generic analyze_subject {
            accept {
                text: String,
            }
            returns Analysis;
            run |input| async move {
                Ok(Analysis {
                    word_count: input.text.split_whitespace().count(),
                    urgent: input.text.contains('!'),
                })
            }
        }

        create intake {
            accept {
                subject: String,
            }
            change set(status = "open");
            change relate_actor(opener_id);
            persist manual;
        }
    }

    policies {
        policy action(open) | action(analyze_subject) | action(intake) {
            authorize_if actor_present
        }
        policy action_type(read) {
            authorize_if relates_to(opener_id)
            authorize_if relates_to(representative_id)
            authorize_if is_nil(representative_id) && actor_eq(role = "representative")
        }
        policy action(assign) {
            authorize_if actor_eq(role = "representative")
        }
        policy action(close) {
            authorize_if relates_to(opener_id)
            authorize_if relates_to(representative_id)
        }
    }
}

pub fn intake_store() -> &'static Mutex<HashMap<Uuid, Ticket>> {
    static STORE: OnceLock<Mutex<HashMap<Uuid, Ticket>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn intake_get(id: Uuid) -> Option<Ticket> {
    intake_store().lock().ok()?.get(&id).cloned()
}

pub fn actor_customer(id: Uuid) -> Actor {
    Actor::new(id).with("role", "customer")
}

pub fn actor_representative(id: Uuid) -> Actor {
    Actor::new(id).with("role", "representative")
}

impl std::fmt::Display for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}  {:<6}  \"{}\"  opener={}  assignee={}",
            &self.id.to_string()[..8],
            self.status.as_str(),
            self.subject,
            &self.opener_id.to_string()[..8],
            self.representative_id
                .map(|id| id.to_string()[..8].to_string())
                .unwrap_or_else(|| "-".into()),
        )
    }
}
