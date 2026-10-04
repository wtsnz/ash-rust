use ash_archival::archival;
use ash_core::CiString;
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::crew::CrewMember;
use super::transponder::Transponder;
use crate::cargo::voyage::Voyage;
use crate::types::ShipClass;
use crate::world::reservation::BerthReservation;

#[archival]
#[state_machine]
resource! {
    /// A ship of the line. Decommissioning archives it and cancels its berth
    /// reservations; its registry code is then free for a new hull.
    Ship {
        table "ships";
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
            registry: CiString;
            name: String;
            class: ShipClass;
            dry_mass_tonnes: i64;
            slot_capacity: i64;
            captain_id: Option<Uuid>;
            version: i64 [version];
        }

        identities {
            identity live_registry: [registry], where: "archived_at IS NULL";
        }

        checks {
            check sane_hull: "dry_mass_tonnes > 0 AND slot_capacity > 0";
        }

        indexes {
            index by_captain: [captain_id], where: "captain_id IS NOT NULL";
        }

        state_machine {
            state_attribute status;
            initial: "docked";
            transition depart, from: ["docked"], to: "in_transit";
            transition arrive, from: ["in_transit"], to: "docked";
            transition begin_maintenance, from: ["docked"], to: "maintenance";
            transition end_maintenance, from: ["maintenance"], to: "docked";
        }

        archive {
            exclude_read_actions [decommissioned];
            archive_related [reservations];
        }

        relationships {
            belongs_to captain: CrewMember [fk: captain_id, on_delete: nilify];
            has_many voyages: Voyage [fk: ship_id];
            has_many reservations: BerthReservation [fk: ship_id];
            has_one transponder: Transponder [fk: ship_id];
        }

        aggregates {
            voyage_count: Option<i64> = count(voyages);
            flights_completed: Option<i64> = count(voyages, filter: status == "completed");
        }

        actions {
            create commission {
                primary;
                accept [registry, name, class, dry_mass_tonnes, slot_capacity, captain_id];
                validate string_length(name, min: 2, max: 60);
            }

            read read {
                primary;
            }

            read decommissioned {
                prepare filter(!archived_at.is_nil());
            }

            update assign_captain {
                accept [captain_id];
            }

            // Each transition checks the stored status. SQLite can't raise that from within
            // an update statement, as AshSqlite can't, so there these read the record first;
            // elsewhere they still run as one statement.
            update depart { require_atomic false; }
            update arrive { require_atomic false; }
            update begin_maintenance { require_atomic false; }
            update end_maintenance { require_atomic false; }

            destroy decommission {
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
            policy action(commission) | action(assign_captain) | action(decommission) | action(begin_maintenance) | action(end_maintenance) {
                authorize_if actor_eq(role = "dispatcher");
            }
            policy action(depart) | action(arrive) {
                authorize_if relates_to_actor(captain_id);
                authorize_if actor_eq(role = "dispatcher");
            }
        }
    }
}
