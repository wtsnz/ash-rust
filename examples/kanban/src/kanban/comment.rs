use super::card::Card;
use ash_core::resource;
use uuid::Uuid;

resource! {
    Comment {
        table "comments";

    attributes {
        id: Uuid [pk];
        card_id: Uuid;
        author_id: Option<Uuid>;
        body: String;
    }

    relationships {
        belongs_to card: Card [fk: card_id];
    }

    actions {
        create create {
            accept [card_id, body];
            validate present(body);
            change relate_actor(author_id);
        }

        read read {
            primary;
        }

        update update_body {
            accept [body];
            validate present(body);
        }

        destroy destroy {}
    }

    policies {
        policy always {
            authorize_if always;
        }
    }
    }}
