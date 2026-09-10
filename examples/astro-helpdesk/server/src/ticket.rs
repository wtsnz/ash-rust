use ash_core::resource;
use uuid::Uuid;

use crate::representative::Representative;

resource! {
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        title: String,
        description: Option<String>,
        status: String [atom: "OPEN,IN_PROGRESS,RESOLVED,CLOSED"],
        priority: i64,
        author_id: Option<Uuid>,
    }

    relationships {
        belongs_to author: Representative [fk: author_id],
    }

    actions {
        create open {
            primary
            accept [title, description, status, priority, author_id]
            validate present(title);
            validate string_length(title, min = 5, max = 100);
            validate one_of(status, ["OPEN", "IN_PROGRESS", "RESOLVED", "CLOSED"]);
            validate numericality(priority, min = 1, max = 5);
        }

        update change_status {
            primary
            accept [status]
            validate one_of(status, ["OPEN", "IN_PROGRESS", "RESOLVED", "CLOSED"]);
        }

        update update_details {
            accept [title, description, priority]
            validate present(title);
            validate string_length(title, min = 5, max = 100);
            validate numericality(priority, min = 1, max = 5);
        }

        destroy close {
            primary
        }

        read read {
            primary
        }
    }
}
