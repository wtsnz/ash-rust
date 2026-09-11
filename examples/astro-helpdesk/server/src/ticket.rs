use ash_core::{AshEnum, resource};
use uuid::Uuid;

use crate::representative::Representative;

#[derive(AshEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TicketStatus {
    #[ash(string = "OPEN")]
    Open,
    #[ash(string = "IN_PROGRESS")]
    InProgress,
    #[ash(string = "RESOLVED")]
    Resolved,
    #[ash(string = "CLOSED")]
    Closed,
}

resource! {
    Ticket {
        table "tickets";

        attributes {
            id: Uuid [pk];
            title: String;
            description: Option<String>;
            status: TicketStatus [enum];
            priority: i64;
            author_id: Option<Uuid>;
        }

        relationships {
            belongs_to author: Representative [fk: author_id];
        }

        actions {
            create open {
                primary;
                accept [title, description, status, priority, author_id];
                validate present(title);
                validate string_length(title, min: 5, max: 100);
                validate numericality(priority, min: 1, max: 5);
            }

            update change_status {
                primary;
                accept [status];
            }

            update update_details {
                accept [title, description, priority];
                validate present(title);
                validate string_length(title, min: 5, max: 100);
                validate numericality(priority, min: 1, max: 5);
            }

            destroy close {
                primary;
            }

            read read {
                primary;
            }
        }
    }
}
