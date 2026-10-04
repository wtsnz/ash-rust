use ash_core::{FieldMap, Filter, Value};
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::agent::Agent;
use super::audit_event::AuditEvent;
use super::comment::Comment;
use super::tag::Tag;
use super::ticket_tag::TicketTag;

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

        indexes {
            // Foreign keys, indexed as a real app indexes them (AshPostgres creates none).
            index by_assignee: [assignee_id];
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

            // Each ticket it finds notes the start of its id, as metadata.
            read noted {
                pagination keyset: true, countable: true, required: false;
                metadata short_id: String;
                prepare after_action(note_short_ids);
            }

            create open {
                primary;
                accept [subject, body, priority, confidential, requester_email];
                argument comments: Option<Vec<FieldMap>>;
                metadata comments_given: i64;
                validate present(requester_email);
                validate string_length(subject, min: 3, max: 200);
                validate numericality(priority, min: 1, max: 4);
                change relate_actor(author_id);
                change manage_relationship(comments, create);
                change func(note_comments_given);
            }

            update assign {
                accept [assignee_id];
            }

            update start {}

            update hold {}

            update resolve {}

            update reopen {
                change atomic_update(reopen_count, reopen_count + 1);
            }

            update close {}

            update view {
                change atomic_update(view_count, view_count + 1);
            }

            update edit {
                accept [subject, priority];
                validate string_length(subject, min: 3, max: 200);
                validate numericality(priority, min: 1, max: 4);
            }

            /// Opens a ticket with its comments, assigns it to the active agent or admin
            /// with the fewest open tickets (then by name), and records it, in one
            /// transaction.
            generic route {
                transaction;
                argument subject: String;
                argument body: String;
                argument priority: i64;
                argument requester_email: String;
                argument comments: Option<Vec<FieldMap>>;
                returns Uuid;
                run |input| async move {
                    let ctx = input.ctx;
                    let ticket = Ticket::open(ctx)
                        .subject(input.subject)
                        .body(input.body)
                        .priority(input.priority)
                        .requester_email(input.requester_email)
                        .comments(input.comments)
                        .await?;
                    let staff = Filter::in_list("role", vec![Value::from("agent"), Value::from("admin")]);
                    let agent = Agent::query(ctx)
                        .filter(Filter::And(vec![Filter::eq("active", true), staff]))
                        .load_aggregate(Agent::open_assigned)
                        .sort(Agent::open_assigned)
                        .sort(Agent::name)
                        .first()
                        .await?;
                    let ticket = match agent {
                        Some(agent) => ticket.assign_on(ctx).assignee_id(Some(agent.id)).await?,
                        None => ticket,
                    };
                    AuditEvent::record(ctx).ticket_id(ticket.id).kind("routed".to_string()).await?;
                    Ok(ticket.id)
                };
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

/// Each ticket a read finds notes the start of its id, as its `short_id` metadata.
fn note_short_ids(_arguments: &FieldMap, records: &mut [FieldMap]) -> ash_core::Result<()> {
    for record in records {
        if let Some(id) = record.get("id").and_then(Value::as_uuid) {
            ash_core::put_metadata(record, "short_id", id.to_string()[..8].to_string());
        }
    }
    Ok(())
}

/// An opened ticket notes how many comments it was opened with, as its
/// `comments_given` metadata.
fn note_comments_given(ctx: &mut ash_core::ChangeContext<'_>) -> ash_core::Result<()> {
    let given = ctx.arguments.get("comments").and_then(Value::as_array).map_or(0, <[Value]>::len) as i64;
    ctx.after_action(move |record| {
        ash_core::put_metadata(record, "comments_given", given);
        Ok(())
    });
    Ok(())
}
