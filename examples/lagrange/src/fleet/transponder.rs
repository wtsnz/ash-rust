use ash_authentication::authentication;
use uuid::Uuid;

use super::ship::Ship;

#[authentication]
resource! {
    /// The box on a ship that reports its position. It signs in with an API key, and
    /// its `kind` and `ship_id` reach telemetry policies through the actor.
    Transponder {
        table "transponders";
        timestamps;

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
            ship_id: Uuid;
            label: String;
            kind: String = "transponder";
            api_key_hash: Option<String>;
        }

        identities {
            identity one_per_ship: [ship_id], message: "that ship already has a transponder";
        }

        relationships {
            belongs_to ship: Ship [fk: ship_id, on_delete: cascade];
        }

        authentication {
            strategy api_key {
                api_key_field: api_key_hash;
                key_prefix: "lgx_";
            }
        }

        actions {
            create install {
                primary;
                accept [ship_id, label, api_key_hash];
            }

            read read {
                primary;
            }
        }

        policies {
            bypass {
                authorize_if actor_eq(role = "port_authority");
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
            policy action(install) {
                authorize_if actor_eq(role = "dispatcher");
            }
        }
    }
}
