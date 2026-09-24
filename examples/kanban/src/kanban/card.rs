use super::board::Board;
use super::checklist_item::ChecklistItem;
use super::comment::Comment;
use super::list::List;
use ash_core::{Binary, CiString, Date, Float, resource};
use uuid::Uuid;

resource! {
    Card {
        table "cards";

    attributes {
        id: Uuid [pk];
        board_id: Uuid;
        list_id: Uuid;
        title: String;
        description: Option<String>;
        position: i64;
        estimate: Option<Float>;
        due_on: Option<Date>;
        attachment: Option<Binary>;
        requester_email: Option<CiString>;
        archived: bool;
        creator_id: Option<Uuid>;
        assignee_id: Option<Uuid>;
    }

    relationships {
        belongs_to list: List [fk: list_id];
        belongs_to board: Board [fk: board_id];
        has_many checklist_items: ChecklistItem [fk: card_id];
        has_many comments: Comment [fk: card_id];
    }

    calculations {
        title_length: Option<i64> = string_length(title);
    }

    aggregates {
        checklist_count: Option<i64> = count(checklist_items);
        completed_checklist_count: Option<i64> = count(checklist_items, filter: completed == true);
        comment_count: Option<i64> = count(comments);
    }

    statements {
        statement citext only postgres {
            up "CREATE EXTENSION IF NOT EXISTS citext";
            down "DROP EXTENSION IF EXISTS citext";
        }
    }

    actions {
        create create {
            accept [board_id, list_id, title, description, position, estimate, due_on, attachment, requester_email];
            validate present(title);
            validate string_length(title, min: 1);
            change set(archived = false);
            change relate_actor(creator_id);
        }

        read read {
            primary;
        }

        update update_details {
            accept [title, description];
        }

        update move_to_list {
            accept [list_id, position];
        }

        update assign {
            accept [assignee_id];
        }

        update archive {
            change set(archived = true);
        }
    }

    policies {
        policy always {
            authorize_if always;
        }
    }
    }}
