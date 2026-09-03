use super::card::Card;
use super::list::List;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource Board;
    table "boards";

    attributes {
        id: Uuid [pk],
        workspace_id: Uuid,
        name: String,
        description: Option<String>,
        archived: bool,
    }

    relationships {
        has_many lists: Vec<List> [fk: "board_id"],
        has_many cards: Vec<Card> [fk: "board_id"],
    }

    aggregates {
        list_count: Option<i64> = count(lists),
        card_count: Option<i64> = count(cards),
        open_card_count: Option<i64> = count(cards, filter: archived == false),
    }

    actions {
        create create {
            accept {
                workspace_id: Uuid,
                name: String,
                description: Option<String>,
            }
            validate present(name);
            change set(archived = false);
        }

        read read {
            primary
        }

        update update_details {
            accept {
                name: String,
                description: Option<String>,
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
