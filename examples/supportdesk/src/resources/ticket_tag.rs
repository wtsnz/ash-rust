use ash_core::resource;
use uuid::Uuid;

resource! {
    /// A ticket filed under a tag.
    TicketTag {
        table "ticket_tags";

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            ticket_id: Uuid;
            tag_id: Uuid;
        }

        indexes {
            // Foreign keys, indexed as a real app indexes them (AshPostgres creates none).
            index by_ticket: [ticket_id];
        }

        actions {
            read read {
                primary;
                pagination keyset: true, countable: true, required: false;
            }

            create seed {
                accept [id, org, ticket_id, tag_id];
            }
        }
    }
}
