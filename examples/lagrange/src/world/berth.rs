use ash_core::resource;
use uuid::Uuid;

use super::port::Port;
use super::reservation::BerthReservation;
use crate::types::ClampType;

resource! {
    /// A docking slot at a port, known by its code there (`A1`, `HEAVY-2`).
    ///
    /// Reservations point at `(port_id, code)`, so renaming a berth carries its
    /// reservations along. Claiming a berth raises its lock version, which is what
    /// keeps two dispatchers from booking the same window at once.
    Berth {
        table "berths";

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            port_id: Uuid;
            code: String;
            clamp: ClampType;
            max_mass_tonnes: i64;
            version: i64 [version];
        }

        identities {
            identity port_code: [port_id, code], message: "this port already has a berth with that code";
        }

        checks {
            check positive_mass: "max_mass_tonnes > 0";
        }

        relationships {
            belongs_to port: Port [fk: port_id];
            has_many reservations: BerthReservation [fk: [port_id, berth_code], references: [port_id, code]];
        }

        aggregates {
            active_reservations: Option<i64> = count(reservations, filter: status == "active");
        }

        actions {
            create build {
                primary;
                accept [port_id, code, clamp, max_mass_tonnes];
                validate string_length(code, min: 1, max: 12);
            }

            read read {
                primary;
            }

            update recode {
                accept [code];
                validate string_length(code, min: 1, max: 12);
            }

            /// Takes the berth's lock before a reservation is written. Two dispatchers
            /// that read the same version cannot both claim it.
            update claim {}
        }

        policies {
            policy action_type(read) | action(claim) {
                authorize_if actor_present;
            }
            policy action(build) | action(recode) {
                authorize_if actor_eq(role = "port_authority");
            }
        }
    }
}
