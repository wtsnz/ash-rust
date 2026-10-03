use ash_core::resource;
use uuid::Uuid;

resource! {
    /// Something that happened to a ticket, and who did it.
    AuditEvent {
        table "audit_events";
        timestamps [inserted_at, updated_at];

        multitenancy {
            strategy: attribute;
            attribute: org;
        }

        attributes {
            id: Uuid [pk];
            org: String;
            ticket_id: Uuid;
            actor_id: Option<Uuid>;
            kind: String;
        }

        actions {
            read read {
                primary;
                pagination keyset: true, countable: true, required: false;
            }

            create record {
                primary;
                accept [ticket_id, kind];
                change relate_actor(actor_id);
            }
        }
    }
}
