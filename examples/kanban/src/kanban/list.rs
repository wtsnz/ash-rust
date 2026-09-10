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
        belongs_to board: Board [fk: board_id],
        has_many cards: Card [fk: list_id],
    }

    aggregates {
        card_count: Option<i64> = count(cards),
        open_card_count: Option<i64> = count(cards, filter: archived == false),
        has_cards: Option<bool> = exists(cards),
    }

    actions {
        create create {
            accept [board_id, title, position];
            validate present(title);
            validate numericality(position, min = 0);
            change set(archived = false);
        }

        read read {
            primary
        }

        update rename {
            accept [title];
            validate present(title);
        }

        update move_position {
            accept [position];
            validate numericality(position, min = 0);
        }

        update archive {
            change set(archived = true);
        }
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
