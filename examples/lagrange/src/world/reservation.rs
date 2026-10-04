use ash_core::{Error, Result, UtcDateTime, ValidationContext, Value, resource};
use uuid::Uuid;

use super::berth::Berth;
use crate::fleet::ship::Ship;
use crate::types::ReservationStatus;

/// A docking window must end after it starts. Timestamps may carry different offsets,
/// so they are compared as instants rather than as text.
pub fn window_runs_forward(ctx: &ValidationContext<'_>) -> Result<()> {
    let instant = |name: &str| match ctx.fields.get(name) {
        Some(Value::String(raw)) => chrono::DateTime::parse_from_rfc3339(raw).ok(),
        _ => None,
    };
    match (instant("starts_at"), instant("ends_at")) {
        (Some(starts), Some(ends)) if ends <= starts => Err(Error::validation("ends_at", "a docking window must end after it starts", Vec::new())),
        _ => Ok(()),
    }
}

resource! {
    /// A ship's hold on a berth for a docking window.
    ///
    /// Each shipping line sees its own reservations, but the berth is shared, so the
    /// overlap check in [`crate::ops::reserve_berth`] looks across lines. On Postgres an
    /// exclusion constraint backs it up in the database.
    BerthReservation {
        table "berth_reservations";
        timestamps;

        multitenancy {
            strategy: attribute;
            attribute: line;
            global: true;
        }

        actor {
            role: String;
        }

        attributes {
            id: Uuid [pk];
            line: String;
            port_id: Uuid;
            berth_code: String;
            ship_id: Uuid;
            starts_at: UtcDateTime;
            ends_at: UtcDateTime;
            status: ReservationStatus [default: ReservationStatus::Active];
        }

        checks {
            check window: "ends_at > starts_at";
        }

        indexes {
            index active_by_berth: [port_id, berth_code], where: "status = 'active'", include: [starts_at, ends_at];
            index by_ship: [ship_id];
        }

        statements {
            statement btree_gist only postgres {
                up "CREATE EXTENSION IF NOT EXISTS btree_gist WITH SCHEMA public";
            }
            statement no_overlap only postgres {
                up "ALTER TABLE berth_reservations ADD CONSTRAINT berth_reservations_no_overlap EXCLUDE USING gist (port_id WITH =, berth_code WITH =, tstzrange(starts_at, ends_at) WITH &&) WHERE (status = 'active')";
                down "ALTER TABLE berth_reservations DROP CONSTRAINT IF EXISTS berth_reservations_no_overlap";
            }
        }

        relationships {
            belongs_to berth: Berth [fk: [port_id, berth_code], references: [port_id, code], on_update: cascade, on_delete: restrict];
            belongs_to ship: Ship [fk: ship_id];
        }

        actions {
            create reserve {
                primary;
                accept [port_id, berth_code, ship_id, starts_at, ends_at];
                validate func(window_runs_forward);
            }

            read read {
                primary;
            }

            read active {
                prepare filter(status == ReservationStatus::Active);
                prepare sort(starts_at, asc);
            }

            update release {
                change set(status = ReservationStatus::Released);
            }

            destroy cancel {
                primary;
            }
        }

        policies {
            policy always {
                authorize_if actor_present;
            }
        }
    }
}
