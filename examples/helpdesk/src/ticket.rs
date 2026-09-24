use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::representative::Representative;
use ash_core::{Actor, AshEnum, Binary, CiString, Date, Float, resource};
use uuid::Uuid;

#[derive(AshEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Closed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    pub word_count: usize,
    pub urgent: bool,
}

resource! {
    Ticket {
        table "tickets";

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            subject: String;
            status: Status [enum];
            opener_id: Uuid;
            representative_id: Option<Uuid>;
            estimate: Option<Float>;
            due_on: Option<Date>;
            attachment: Option<Binary>;
            requester_email: Option<CiString>;
        }

        statements {
            statement citext only postgres {
                up "CREATE EXTENSION IF NOT EXISTS citext";
                down "DROP EXTENSION IF EXISTS citext";
            }
        }

        relationships {
            belongs_to representative: Representative;
        }

        calculations {
            subject_length: Option<i64> = string_length(subject);
        }

        actions {
            create open {
                accept [subject, estimate, due_on, attachment, requester_email];
                validate present(subject);
                validate string_length(subject, min: 2);
                change set(status = Status::Open);
                change relate_actor(opener_id);
            }

            read read {
                primary;
            }

            update assign {
                accept [representative_id];
            }

            update close {
                change set(status = Status::Closed);
            }

            generic analyze_subject {
                accept {
                    text: String,
                };
                returns Analysis;
                run |input| async move {
                    Ok(Analysis {
                        word_count: input.text.split_whitespace().count(),
                        urgent: input.text.contains('!'),
                    })
                };
            }

            create intake {
                accept [subject];
                change set(status = Status::Open);
                change relate_actor(opener_id);
                persist manual;
            }
        }

        policies {
            policy action(open) | action(analyze_subject) | action(intake) {
                authorize_if actor_present;
            }
            policy action_type(read) {
                authorize_if relates_to(opener_id);
                authorize_if relates_to(representative_id);
                authorize_if is_nil(representative_id) && actor_eq(role = "representative");
            }
            policy action(assign) {
                authorize_if actor_eq(role = "representative");
            }
            policy action(close) {
                authorize_if relates_to(opener_id);
                authorize_if relates_to(representative_id);
            }
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
