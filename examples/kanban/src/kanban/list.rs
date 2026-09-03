use super::board::Board;
use super::card::Card;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource List;
    table "lists";

    attributes {
        id: Uuid [pk],
        board_id: Uuid,
        title: String,
        position: i64,
        archived: bool,
    }

    relationships {
        belongs_to board: Option<Board> [fk: "board_id"],
        has_many cards: Vec<Card> [fk: "list_id"],
    }

    aggregates {
        card_count: Option<i64> = count(cards),
        open_card_count: Option<i64> = count(cards, filter: archived == false),
        has_cards: Option<bool> = exists(cards),
    }

    actions {
        create create {
            accept {
                board_id: Uuid,
                title: String,
                position: i64,
            }
            validate present(title);
            validate numericality(position, min = 0);
            change set(archived = false);
        }

        read read {
            primary
        }

        update rename {
            accept {
                title: String,
            }
            validate present(title);
        }

        update move_position {
            accept {
                position: i64,
            }
            validate numericality(position, min = 0);
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
