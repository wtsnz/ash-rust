use super::card::Card;
use ash_core::resource;
use uuid::Uuid;

resource! {
    resource ChecklistItem;
    table "checklist_items";

    attributes {
        id: Uuid [pk],
        card_id: Uuid,
        title: String,
        completed: bool,
        position: i64,
    }

    relationships {
        belongs_to card: Card [fk: card_id],
    }

    actions {
        create create {
            accept [card_id, title, position];
            validate present(title);
            change set(completed = false);
        }

        read read {
            primary
        }

        update toggle {
            accept [completed];
        }

        destroy destroy {}
    }

    policies {
        policy always {
            authorize_if always
        }
    }
}
