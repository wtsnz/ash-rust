use ash_core::UtcDateTimeUsec;
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::depot::Depot;
use crate::rides::trip::Trip;

#[state_machine]
resource! {
    /// A Cybercab: two seats, no steering wheel, no pedals. It reports where it is, how
    /// fast it's going and how much charge it has every second it's on the road.
    ///
    /// Its status is the dispatch cycle: available, dispatched to a pickup, on a trip,
    /// then available again, with detours back to a hub to charge and out of service.
    Cab {
        table "cabs";
        timestamps;

        attributes {
            id: Uuid [pk];
            /// Fleet call sign, as painted on the door: "CC-0142".
            call_sign: String;
            /// What the night shift calls it.
            nickname: String;
            vin: String;
            software: String;
            depot_id: Uuid;
            lng: f64;
            lat: f64;
            heading_deg: i64;
            speed_kph: i64;
            battery_pct: i64;
            range_km: i64;
            odometer_km: f64;
            cabin_temp_c: f64;
            /// Stopped at the curb by an operator, whatever it was doing.
            halted: bool [default: false];
            /// The trip it's dispatched to or carrying.
            trip_id: Option<Uuid>;
            last_seen_at: Option<UtcDateTimeUsec>;
        }

        identities {
            identity unique_call_sign: [call_sign];
            identity unique_vin: [vin];
        }

        checks {
            check battery_range: "battery_pct BETWEEN 0 AND 100";
        }

        state_machine {
            state_attribute status;
            initial: "available";
            transition dispatch, from: ["available"], to: "dispatched";
            transition begin_ride, from: ["dispatched"], to: "on_trip";
            transition finish_ride, from: ["on_trip"], to: "available";
            transition stand_down, from: ["dispatched"], to: "available";
            transition recall, from: ["available"], to: "returning";
            transition plug_in, from: ["returning"], to: "charging";
            transition unplug, from: ["charging"], to: "available";
            transition ground, from: ["available", "returning", "charging"], to: "maintenance";
            transition release, from: ["maintenance"], to: "available";
        }

        relationships {
            belongs_to depot: Depot [fk: depot_id];
            belongs_to trip: Trip [fk: trip_id];
            has_many trips: Trip [fk: cab_id];
        }

        aggregates {
            trips_completed: Option<i64> = count(trips, filter: status == "completed");
            fares_cents: Option<i64> = sum(trips, fare_cents, filter: status == "completed");
        }

        actions {
            create commission {
                primary;
                accept [call_sign, nickname, vin, software, depot_id, lng, lat, heading_deg, speed_kph, battery_pct, range_km, odometer_km, cabin_temp_c];
            }

            read read {
                primary;
            }

            /// The heartbeat: where the cab is and how it's doing.
            update report {
                primary;
                accept [lng, lat, heading_deg, speed_kph, battery_pct, range_km, odometer_km, cabin_temp_c, last_seen_at];
            }

            update dispatch {
                accept [trip_id];
            }

            update begin_ride {}

            update finish_ride {
                accept [trip_id];
            }

            update stand_down {
                accept [trip_id];
            }

            /// Sends an idle cab back to its hub to charge.
            update recall {}

            update plug_in {}

            update unplug {}

            /// Takes the cab out of service.
            update ground {}

            update release {}

            /// Stops the cab at the curb wherever it is.
            update pull_over {
                change set(halted = true);
            }

            update resume {
                change set(halted = false);
            }
        }
    }
}
