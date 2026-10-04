use ash_core::{CiString, Date, Decimal};
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::container::Container;
use crate::world::port::Port;

#[state_machine]
resource! {
    /// A shipper's booking to move cargo between two ports. Re-importing the same
    /// `external_ref` from the shipper's system updates the booking instead of
    /// duplicating it.
    Contract {
        table "contracts";
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
            external_ref: String;
            shipper_email: CiString;
            origin_port_id: Uuid;
            destination_port_id: Uuid;
            freight_credits: i64;
            insured_value: Decimal;
            deliver_by: Date;
            version: i64 [version];
        }

        identities {
            identity external: [line, external_ref], message: "that booking reference is already on file";
        }

        checks {
            check positive_freight: "freight_credits > 0";
            check distinct_ports: "origin_port_id <> destination_port_id";
        }

        indexes {
            index open_by_destination: [destination_port_id], where: "status <> 'closed' AND status <> 'cancelled'";
        }

        state_machine {
            state_attribute status;
            initial: "booked";
            transition load, from: ["booked"], to: "loaded";
            transition dispatch, from: ["loaded"], to: "in_transit";
            transition deliver, from: ["in_transit"], to: "delivered";
            transition close, from: ["delivered"], to: "closed";
            transition cancel, from: ["booked"], to: "cancelled";
        }

        relationships {
            belongs_to origin: Port [fk: origin_port_id];
            belongs_to destination: Port [fk: destination_port_id];
            has_many containers: Container [fk: contract_id, on_delete: restrict];
        }

        calculations {
            rush_quote(surcharge_pct: i64): i64 = freight_credits + freight_credits * arg(surcharge_pct) / 100;
        }

        aggregates {
            container_count: Option<i64> = count(containers);
            booked_mass: Option<i64> = sum(containers, mass_tonnes);
        }

        actions {
            create book {
                primary;
                accept [external_ref, shipper_email, origin_port_id, destination_port_id, freight_credits, insured_value, deliver_by];
                validate string_length(external_ref, min: 3, max: 40);
                validate numericality(freight_credits, min: 1);
            }

            read read {
                primary;
            }

            read open {
                prepare filter(status != "closed");
                prepare filter(status != "cancelled");
                prepare sort(deliver_by, asc);
            }

            // Each transition checks the stored status. SQLite can't raise that from within
            // an update statement, as AshSqlite can't, so there these read the record first;
            // elsewhere they still run as one statement.
            update load { require_atomic false; }
            update dispatch { require_atomic false; }
            update deliver { require_atomic false; }
            update close { require_atomic false; }
            update cancel { require_atomic false; }
        }

        policies {
            bypass {
                authorize_if actor_eq(role = "port_authority");
            }
            policy action_type(read) {
                authorize_if actor_present;
            }
            policy action(book) | action(cancel) {
                authorize_if actor_eq(role = "shipper");
                authorize_if actor_eq(role = "dispatcher");
            }
            policy action(load) | action(close) {
                authorize_if actor_eq(role = "dispatcher");
            }
            // The captain reports the cargo away and delivered as the ship flies.
            policy action(dispatch) | action(deliver) {
                authorize_if actor_eq(role = "dispatcher");
                authorize_if actor_eq(role = "captain");
            }
        }
    }
}
