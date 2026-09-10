use ash_core::{ActionTarget, Context, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    /// Ticket resource representation
    resource Ticket;
    table "tickets";

    attributes {
        id: Uuid [pk],
        title: String,
        status: String,
        assignee: Option<String>,
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
            accept [assignee];
            change set(status = "in_progress");
        }

        /// Closes and deletes a ticket
        destroy close {
            primary;
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
