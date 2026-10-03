use ash_core::FieldMap;
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::agent::Agent;
use super::comment::Comment;
use super::tag::Tag;
use super::ticket_tag::TicketTag;
use crate::changes::{COUNT_REOPEN, COUNT_VIEW};

#[state_machine]
resource! {
    /// A customer's request, and the desk's work on it.
    Ticket {
        table "tickets";
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
            subject: String;
            body: String;
            /// 1 (low) to 4 (urgent).
            priority: i64;
            /// Hidden from viewers.
            confidential: bool [default: false];
            /// Only an admin and the ticket's assignee see it. Optional in the type because a
            /// redacted read holds null (see GAPS.md); `open` requires it.
            requester_email: Option<String>;
            assignee_id: Option<Uuid>;
            author_id: Option<Uuid>;
            view_count: i64 [default: 0];
            reopen_count: i64 [default: 0];
            version: i64 [version];
        }

        state_machine {
            state_attribute status;
            initial: "new";
            transition start, from: ["new"], to: "open";
            transition hold, from: ["open"], to: "pending";
            transition resolve, from: ["open", "pending"], to: "resolved";
            transition reopen, from: ["resolved"], to: "open";
            transition close, from: ["resolved"], to: "closed";
        }

        relationships {
            belongs_to assignee: Agent [fk: assignee_id];
            belongs_to author: Agent [fk: author_id];
            has_many comments: Comment [fk: ticket_id];
            many_to_many tags: Tag [through: TicketTag, source_fk: ticket_id, dest_fk: tag_id];
        }

        aggregates {
            comment_count: Option<i64> = count(comments);
            public_comment_count: Option<i64> = count(comments, filter: internal == false);
            has_internal_notes: Option<bool> = exists(comments, filter: internal == true);
        }

        calculations {
            weight: i64 = priority * 10;
            subject_length: Option<i64> = string_length(subject);
            scaled_priority(factor: i64): i64 = priority * arg(factor);
        }

        policies {
            bypass {
                authorize_if actor_attribute_equals(role, "admin");
            }

            policy action_type(read) {
                authorize_if actor_attribute_equals(role, "agent");
                authorize_if eq(confidential, false);
            }

            policy action_type(create) {
                authorize_if actor_attribute_equals(role, "agent");
            }

            policy action_type(update) {
                authorize_if actor_attribute_equals(role, "agent");
            }

            policy action_type(destroy) {
                authorize_if actor_attribute_equals(role, "agent");
            }

            policy action(route) {
                authorize_if actor_attribute_equals(role, "agent");
            }
        }

        field_policies {
            field requester_email {
                authorize_if relates_to_actor(assignee_id);
                authorize_if actor_attribute_equals(role, "admin");
            }
        }

        actions {
            read read {
                primary;
                pagination keyset: true, countable: true, required: false;
            }

            create open {
                primary;
                accept [subject, body, priority, confidential, requester_email];
                // Optional, where Ash's defaults to `[]`: ash-rust's arguments take no default.
                argument comments: Option<Vec<FieldMap>>;
                validate present(requester_email);
                validate string_length(subject, min: 3, max: 200);
                validate numericality(priority, min: 1, max: 4);
                change relate_actor(author_id);
                change manage_relationship(comments, create);
            }

            update assign {
                accept [assignee_id];
            }

            update start {}

            update hold {}

            update resolve {}

            update reopen {
                change custom(&COUNT_REOPEN);
            }

            update close {}

            update view {
                change custom(&COUNT_VIEW);
            }

            update edit {
                accept [subject, priority];
                validate string_length(subject, min: 3, max: 200);
                validate numericality(priority, min: 1, max: 4);
            }

            /// Opens a ticket with its comments, assigns it to the active agent with the
            /// fewest open tickets, and records it, in one transaction. The server
            /// supplies the work (`.run(...)`), on a data layer that can transact.
            generic route {
                argument subject: String;
                argument body: String;
                argument priority: i64;
                argument requester_email: String;
                argument comments: Option<Vec<FieldMap>>;
                returns Uuid;
            }

            destroy destroy {
                primary;
            }

            create seed {
                accept [id, org, subject, body, priority, confidential, requester_email, assignee_id, author_id, status, view_count, reopen_count, version, inserted_at, updated_at];
            }
        }
    }
}
