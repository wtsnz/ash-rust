use ash_core::resource;
use uuid::Uuid;

use super::agent::Agent;
use super::ticket::Ticket;

resource! {
    /// A reply on a ticket, or an `internal` note only agents see.
    Comment {
        table "comments";
        timestamps [inserted_at, updated_at];

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            ticket_id: Uuid;
            author_id: Option<Uuid>;
            body: String;
            internal: bool [default: false];
        }

        relationships {
            belongs_to ticket: Ticket [fk: ticket_id];
            belongs_to author: Agent [fk: author_id];
        }

        policies {
            bypass {
                authorize_if actor_attribute_equals(role, "admin");
            }

            policy action_type(read) {
                authorize_if actor_attribute_equals(role, "agent");
                authorize_if eq(internal, false);
            }

            policy action_type(create) {
                authorize_if actor_attribute_equals(role, "agent");
            }
        }

        actions {
            read read {
                primary;
            }

            create create {
                primary;
                accept [ticket_id, body, internal];
                change relate_actor(author_id);
            }

            create seed {
                accept [id, org, ticket_id, author_id, body, internal, inserted_at, updated_at];
            }
        }
    }
}
