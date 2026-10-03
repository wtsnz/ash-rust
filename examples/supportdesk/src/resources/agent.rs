use ash_core::resource;
use uuid::Uuid;

use super::ticket::Ticket;

resource! {
    /// Someone who works the desk: an `admin`, an `agent`, or a read-only `viewer`.
    Agent {
        table "agents";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            name: String;
            email: String;
            role: String;
            active: bool [default: true];
        }

        identities {
            identity unique_email: [org, email];
        }

        relationships {
            has_many assigned_tickets: Ticket [fk: assignee_id];
        }

        aggregates {
            /// Open tickets assigned to the agent: what routing balances.
            open_assigned: Option<i64> = count(assigned_tickets, filter: status == "open");
        }

        actions {
            read read {
                primary;
            }

            create seed {
                accept [id, org, name, email, role, active];
            }
        }
    }
}
