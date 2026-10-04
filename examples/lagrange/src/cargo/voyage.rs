use ash_core::{Date, Float, UtcDateTime};
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::container::Container;
use super::stowage::Stowage;
use crate::fleet::ship::Ship;
use crate::world::port::Port;

#[state_machine]
resource! {
    /// One ship's trip from one port to another, with the containers stowed aboard.
    ///
    /// Customs clears a planned voyage once its dangerous goods are sealed. Only the
    /// captain launches it, and only once a berth is held at the destination.
    Voyage {
        table "voyages";
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
            captain_id: Uuid;
            origin_port_id: Uuid;
            destination_port_id: Uuid;
            launch_window: Date;
            distance_au: Float;
            transit_hours: i64;
            reservation_id: Option<Uuid>;
            departed_at: Option<UtcDateTime>;
            arrived_at: Option<UtcDateTime>;
            version: i64 [version];
        }

        checks {
            check distinct_ports: "origin_port_id <> destination_port_id";
            check arrives_after_departure: "arrived_at IS NULL OR departed_at IS NULL OR arrived_at > departed_at";
        }

        indexes {
            index in_flight: [ship_id], where: "status = 'in_transit'";
            index by_window: [launch_window], using: brin;
        }

        state_machine {
            state_attribute status;
            initial: "planned";
            transition clear_customs, from: ["planned"], to: "cleared";
            transition launch, from: ["cleared"], to: "in_transit";
            transition arrive, from: ["in_transit"], to: "arrived";
            transition complete, from: ["arrived"], to: "completed";
            transition scrub, from: ["planned", "cleared"], to: "scrubbed";
        }

        relationships {
            belongs_to ship: Ship [fk: ship_id];
            belongs_to origin: Port [fk: origin_port_id];
            belongs_to destination: Port [fk: destination_port_id];
            has_many stowage: Stowage [fk: voyage_id, on_delete: cascade];
            many_to_many containers: Container [through: Stowage, source_fk: voyage_id, dest_fk: container_id];
        }

        aggregates {
            container_count: Option<i64> = count(containers);
            loaded_mass: Option<i64> = sum(containers, mass_tonnes);
            carries_hazmat: Option<bool> = exists(containers, filter: hazardous == true);
        }

        actions {
            create plan {
                primary;
                accept [ship_id, captain_id, origin_port_id, destination_port_id, launch_window, distance_au, transit_hours];
            }

            read read {
                primary;
            }

            read departures {
                prepare filter(status == "cleared");
                prepare sort(launch_window, asc);
            }

            update hold_berth {
                accept [reservation_id];
            }

            // Each transition checks the stored status. SQLite can't raise that from within
            // an update statement, as AshSqlite can't, so there these read the voyage first;
            // elsewhere they still run as one statement.
            update clear_customs { require_atomic false; }

            update launch {
                accept [departed_at];
                require_atomic false;
            }

            update arrive {
                accept [arrived_at];
                require_atomic false;
            }

            update complete { require_atomic false; }
            update scrub { require_atomic false; }

            destroy discard {
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
            policy action(plan) | action(hold_berth) | action(complete) | action(scrub) | action(discard) {
                authorize_if actor_eq(role = "dispatcher");
            }
            policy action(clear_customs) {
                authorize_if actor_eq(role = "customs");
            }
            policy action(launch) {
                forbid_if is_nil(reservation_id);
                authorize_if relates_to_actor(captain_id);
            }
            policy action(arrive) {
                authorize_if relates_to_actor(captain_id);
                authorize_if actor_eq(role = "dispatcher");
            }
        }
    }
}
