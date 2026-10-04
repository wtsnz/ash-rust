use ash_core::{CiString, Inet, resource};
use uuid::Uuid;

use super::berth::Berth;
use super::planet::Planet;
use crate::types::PortKind;

resource! {
    /// A spaceport on a planet's surface or a station in its orbit.
    Port {
        table "ports";
        timestamps;

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            planet_id: Uuid;
            code: CiString;
            name: String;
            kind: PortKind;
            /// Traffic control's relay, which transponders report to.
            relay: Inet;
        }

        identities {
            identity unique_code: [code], message: "port codes are unique across the system";
        }

        indexes {
            index by_planet: [planet_id], include: [name];
            index by_kind: [kind], using: hash;
        }

        relationships {
            belongs_to planet: Planet [fk: planet_id, on_delete: restrict];
            has_many berths: Berth [fk: port_id, on_delete: restrict];
        }

        calculations {
            label: Option<String> = concat(code, " ", name);
        }

        aggregates {
            berth_count: Option<i64> = count(berths);
        }

        actions {
            create open {
                primary;
                accept [planet_id, code, name, kind, relay];
                validate string_length(name, min: 3, max: 80);
            }

            read read {
                primary;
            }

            update rename {
                primary;
                accept [name];
            }

            update move_relay {
                accept [relay];
            }
        }

        policies {
            policy action_type(read) {
                authorize_if always;
            }
            policy action_type(create) | action_type(update) {
                authorize_if actor_eq(role = "port_authority");
            }
        }
    }
}
