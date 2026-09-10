use ash_core::resource;
use uuid::Uuid;

use crate::ticket::Ticket;

resource! {
    resource Representative;
    table "representatives";

    attributes {
        id: Uuid [pk],
        name: String,
    }

    relationships {
        has_many tickets: Ticket [fk: representative_id],
    }

    aggregates {
        ticket_count: Option<i64> = count(tickets),
        open_ticket_count: Option<i64> = count(tickets, filter: status == "open"),
        has_tickets: Option<bool> = exists(tickets),
        first_ticket_subject: Option<String> = first(tickets, subject),
    }

    actions {
        create create {
            accept [name];
            validate present(name);
            validate string_length(name, min = 2);
        }
        read read {
            primary
        }
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
