use super::board::Board;
use super::checklist_item::ChecklistItem;
use super::comment::Comment;
use super::list::List;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource Card;
    table "cards";

    attributes {
        id: Uuid [pk],
        board_id: Uuid,
        list_id: Uuid,
        title: String,
        description: Option<String>,
        position: i64,
        archived: bool,
        creator_id: Option<Uuid>,
        assignee_id: Option<Uuid>,
    }

    relationships {
        belongs_to list: Option<List> [fk: "list_id"],
        belongs_to board: Option<Board> [fk: "board_id"],
        has_many checklist_items: Vec<ChecklistItem> [fk: "card_id"],
        has_many comments: Vec<Comment> [fk: "card_id"],
    }

    calculations {
        title_length: Option<i64> = "string_length(title)",
    }

    aggregates {
        checklist_count: Option<i64> = count(checklist_items),
        completed_checklist_count: Option<i64> = count(checklist_items, filter: completed == true),
        comment_count: Option<i64> = count(comments),
    }

    actions {
        create create {
            accept {
                board_id: Uuid,
                list_id: Uuid,
                title: String,
                description: Option<String>,
                position: i64,
            }
            validate present(title);
            validate string_length(title, min = 1);
            change set(archived = false);
            change relate_actor(creator_id);
        }

        read read {
            primary
        }

        update update_details {
            accept {
                title: String,
                description: Option<String>,
            }
        }

        update move_to_list {
            accept {
                list_id: Uuid,
                position: i64,
            }
        }

        update assign {
            accept {
                assignee_id: Option<Uuid>,
            }
        }

        update archive {
            changes [
                set(archived = true),
            ]
        }
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
