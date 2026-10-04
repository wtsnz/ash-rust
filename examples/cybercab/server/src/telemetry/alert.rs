use ash_core::UtcDateTimeUsec;
use ash_state_machine::state_machine;
use uuid::Uuid;

use crate::fleet::cab::Cab;
use crate::rides::trip::Trip;
use crate::types::{AlertKind, Severity};

#[state_machine]
resource! {
    /// Something a cab reported that a person should look at: a rider pressing the
    /// assist button, a hard stop, a sensor running degraded. An operator acknowledges
    /// it, then resolves it.
    FleetAlert {
        table "fleet_alerts";

        attributes {
            id: Uuid [pk];
            cab_id: Uuid;
            trip_id: Option<Uuid>;
            kind: AlertKind [enum];
            severity: Severity [enum];
            message: String;
            lng: f64;
            lat: f64;
            raised_at: UtcDateTimeUsec;
            acknowledged_at: Option<UtcDateTimeUsec>;
            resolved_at: Option<UtcDateTimeUsec>;
            /// Who handled it, or "auto" when the cab cleared it itself.
            handled_by: Option<String>;
        }

        indexes {
            index open_by_raised: [raised_at], where: "status <> 'resolved'";
        }

        state_machine {
            state_attribute status;
            initial: "open";
            transition acknowledge, from: ["open"], to: "acknowledged";
            transition resolve, from: ["open", "acknowledged"], to: "resolved";
        }

        relationships {
            belongs_to cab: Cab [fk: cab_id];
            belongs_to trip: Trip [fk: trip_id];
        }

        actions {
            create raise {
                primary;
                accept [cab_id, trip_id, kind, severity, message, lng, lat, raised_at];
            }

            read read {
                primary;
                pagination keyset: true, offset: true, countable: true, required: false;
            }

            update acknowledge {
                accept [acknowledged_at, handled_by];
            }

            update resolve {
                accept [resolved_at, handled_by];
            }

            destroy prune {
                primary;
            }
        }
    }
}
