use ash_core::resource;
use uuid::Uuid;

use super::container::Container;
use super::voyage::Voyage;

resource! {
    /// Where a container rides on a voyage: its bay, stack and tier in the hold.
    Stowage {
        table "stowage";

        multitenancy {
            strategy: attribute;
            attribute: line;
        }

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            line: String;
            voyage_id: Uuid;
            container_id: Uuid;
            bay: i64;
            stack: i64;
            tier: i64;
        }

        identities {
            identity slot: [voyage_id, bay, stack, tier], message: "that slot is already taken";
            identity once_per_voyage: [voyage_id, container_id], message: "that container is already aboard";
        }

        checks {
            check slot_in_hold: "bay BETWEEN 1 AND 40 AND stack BETWEEN 1 AND 16 AND tier BETWEEN 1 AND 8";
        }

        relationships {
            belongs_to voyage: Voyage [fk: voyage_id, on_delete: cascade];
            belongs_to container: Container [fk: container_id, on_delete: restrict];
        }

        actions {
            create stow {
                primary;
                accept [voyage_id, container_id, bay, stack, tier];
            }

            read read {
                primary;
            }

            destroy unstow {
                primary;
            }
        }

        policies {
            policy action_type(read) {
                authorize_if actor_present;
            }
            policy action(stow) | action(unstow) {
                authorize_if actor_eq(role = "dispatcher");
            }
        }
    }
}
