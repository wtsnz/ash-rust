use ash_core::{ActionTarget, Context, resource};
use ash_memory::Memory;
use uuid::Uuid;

mod engineer {
    use super::*;

    resource! {
        /// Engineer who can be assigned tickets
        resource Engineer;
        table "engineers";

        attributes {
            id: Uuid [pk],
            /// Display name for the engineer
            name: String,
        }

        actions {
            create create {
                primary;
                accept [name];
            }

            read read {
                primary;
            }
        }
    }
}

use engineer::Engineer;

resource! {
    /// Ticket resource representation
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        /// Short summary shown in the queue
        title: String,
        status: String,
        assignee: Option<String>,
        engineer_id: Option<Uuid>,
    }

    relationships {
        belongs_to engineer: Option<Engineer> [fk: engineer_id];
    }

    actions {
        /// Creates a new ticket
        create create {
            primary;
            accept [title];
            change set(status = "open");
        }

        /// Primary read action
        read read {
            primary;
        }

        /// Assigns a ticket to an engineer
        update assign {
            accept [assignee, engineer_id];
            change set(status = "in_progress");
        }

        /// Closes and deletes a ticket
        destroy close {
            primary;
        }

        generic summarize {
            /// Extra notes included in the summary
            argument notes: String;
            returns String;
            run |input| async move { Ok(input.notes) };
        }
    }
}

#[tokio::test]
async fn test_action_target_conversions() {
    let id = Uuid::new_v4();
    let target_from_id: ActionTarget<Ticket> = id.into();
    assert_eq!(target_from_id.id(), id);

    let target_from_ref_id: ActionTarget<Ticket> = (&id).into();
    assert_eq!(target_from_ref_id.id(), id);

    let ticket = Ticket {
        id,
        title: "Test".to_string(),
        status: "open".to_string(),
        assignee: None,
        engineer_id: None,
        engineer: Default::default(),
    };

    let target_from_ref: ActionTarget<Ticket> = (&ticket).into();
    assert_eq!(target_from_ref, ActionTarget::Record(ticket.clone()));
    assert_eq!(target_from_ref.id(), id);

    let mut ticket_mut = ticket.clone();
    let target_from_mut_ref: ActionTarget<Ticket> = (&mut ticket_mut).into();
    assert_eq!(target_from_mut_ref, ActionTarget::Record(ticket.clone()));
    assert_eq!(target_from_mut_ref.id(), id);

    let target_from_owned: ActionTarget<Ticket> = ticket.clone().into();
    assert_eq!(target_from_owned, ActionTarget::Record(ticket));
    assert_eq!(target_from_owned.id(), id);
}

#[tokio::test]
async fn test_inherent_action_invocations_without_trait_imports() {
    let ctx = Context::new(Memory::new());

    // 1. Inherent create without trait imports
    let ticket = Ticket::create(&ctx).title("Network down").await.unwrap();
    assert_eq!(ticket.title, "Network down");
    assert_eq!(ticket.status, "open");

    // 2. Inherent update by ID (Uuid into ActionTarget) without trait imports
    let updated_by_id = Ticket::assign(&ctx, ticket.id)
        .assignee(Some("Alice".to_string()))
        .await
        .unwrap();
    assert_eq!(updated_by_id.status, "in_progress");
    assert_eq!(updated_by_id.assignee, Some("Alice".to_string()));

    // 3. Inherent update by reference (&Ticket into ActionTarget) without trait imports
    let updated_by_ref = Ticket::assign(&ctx, &updated_by_id)
        .assignee(Some("Bob".to_string()))
        .await
        .unwrap();
    assert_eq!(updated_by_ref.assignee, Some("Bob".to_string()));

    // 4. Inherent update by owned record (Ticket into ActionTarget) without trait imports
    let updated_by_owned = Ticket::assign(&ctx, updated_by_ref)
        .assignee(Some("Charlie".to_string()))
        .await
        .unwrap();
    assert_eq!(updated_by_owned.assignee, Some("Charlie".to_string()));

    // 5. Inherent destroy by reference (&Ticket into ActionTarget) without trait imports
    Ticket::close(&ctx, &updated_by_owned).await.unwrap();

    // Verify it was destroyed
    assert!(Ticket::get(&ctx, updated_by_owned.id).await.is_err());
}

mod permissive {
    use super::*;

    resource! {
        resource PermissiveTicket;
        table "permissive_tickets";

        attributes {
            id: Uuid [pk],
            title: String,
            status: String,
        };

        relationships {};

        policies {};

        identities {
            identity by_title: [title,];
        };

        calculations {
            title_len: i64 = string_length(title);
        };

        actions {
            create create {
                primary;
                accept [title,];
                change set(status = "open");
                validate one_of(status, ["open", "closed",]);
            };

            read read {
                primary;
            };
        };
    }
}

use permissive::PermissiveTicket;

mod comment {
    use super::*;

    resource! {
        resource Comment;
        table "dx_comments";

        attributes {
            id: Uuid [pk],
            post_id: Uuid,
            body: String,
            amount: i64,
        }

        actions {
            create create {
                accept [post_id, body, amount];
            }

            read read {
                primary;
            }
        }
    }
}

mod post {
    use super::comment::Comment;
    use super::*;

    resource! {
        resource Post;
        table "dx_posts";

        attributes {
            id: Uuid [pk],
            title: String,
        }

        relationships {
            has_many comments: Vec<Comment> [fk: post_id];
        }

        aggregates {
            first_body: Option<String> = first(comments, body);
            total_amount: Option<i64> = sum(comments, amount);
        }

        calculations {
            title_len: i64 = string_length(title);
        }

        actions {
            create create {
                accept [title];
                validate string_length(title, min = 1);
            }

            read read {
                primary;
            }

            generic preview {
                argument suffix: String;
                returns String;
                run |input| async move { Ok(input.suffix) };
            }
        }
    }
}

use comment::Comment;
use post::Post;

#[test]
fn test_field_constants_and_relationship_probe_compile() {
    let _ = Ticket::title;
    let _ = ticket_fields::title;
    let _ = Ticket::engineer;
    let _ = Engineer::name;
}

#[tokio::test]
async fn test_documented_builder_setters_and_belongs_to_fk() {
    let ctx = Context::new(Memory::new());

    let engineer = Engineer::create(&ctx).name("Ada").await.unwrap();
    let ticket = Ticket::create(&ctx).title("Printer jam").await.unwrap();
    assert_eq!(ticket.title, "Printer jam");
    assert!(ticket.engineer_id.is_none());

    let updated = Ticket::assign(&ctx, ticket.id)
        .assignee(Some("Ada".to_string()))
        .engineer_id(Some(engineer.id))
        .await
        .unwrap();
    assert_eq!(updated.assignee, Some("Ada".to_string()));
    assert_eq!(updated.engineer_id, Some(engineer.id));

    let summary = Ticket::summarize(&ctx).notes("needs toner").await.unwrap();
    assert_eq!(summary, "needs toner");
}

#[tokio::test]
async fn test_permissive_punctuation_and_empty_sections() {
    let ctx = Context::new(Memory::new());

    let ticket = PermissiveTicket::create(&ctx)
        .title("Permissive syntax")
        .await
        .unwrap();
    assert_eq!(ticket.title, "Permissive syntax");
    assert_eq!(ticket.status, "open");

    let fetched = PermissiveTicket::get(&ctx, ticket.id).await.unwrap();
    assert_eq!(fetched.title, ticket.title);
}

#[tokio::test]
async fn test_destination_aggregates_and_generic_run_probe() {
    let ctx = Context::new(Memory::new());

    let post = Post::create(&ctx).title("Hello").await.unwrap();
    assert_eq!(post.title, "Hello");

    let comment = Comment::create(&ctx)
        .post_id(post.id)
        .body("Nice post")
        .amount(3)
        .await
        .unwrap();
    assert_eq!(comment.body, "Nice post");

    let preview = Post::preview(&ctx).suffix("!").await.unwrap();
    assert_eq!(preview, "!");

    let _ = Post::title;
    let _ = Post::comments;
    let _ = Comment::body;
    let _ = Comment::amount;
}
