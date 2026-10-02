use ash_core::resource;
use uuid::Uuid;

use super::trip::Trip;

resource! {
    /// A part of the city demand and pricing are tracked in. Operators can declare an
    /// event in a zone (a show at the Moody Center, a race at COTA), which draws riders
    /// and lifts the surge there.
    ServiceZone {
        table "service_zones";

        attributes {
            id: Uuid [pk];
            code: String;
            name: String;
            lng: f64;
            lat: f64;
            radius_m: i64;
            /// Requests per minute the zone sees on an ordinary evening.
            base_demand: f64;
            surge: f64;
            /// Riders waiting for a cab right now.
            waiting: i64 [default: 0];
            event_name: Option<String>;
            /// How many times its usual demand the event brings.
            event_boost: f64;
        }

        identities {
            identity unique_code: [code];
        }

        relationships {
            has_many trips: Trip [fk: zone_id];
        }

        aggregates {
            trips_today: Option<i64> = count(trips);
            completed_today: Option<i64> = count(trips, filter: status == "completed");
        }

        actions {
            create chart {
                primary;
                accept [code, name, lng, lat, radius_m, base_demand, surge, event_boost];
            }

            read read {
                primary;
            }

            /// The dispatcher's view of demand: how many wait, and the surge it calls for.
            update measure {
                primary;
                accept [waiting, surge];
            }

            /// Declares an event in the zone, drawing riders there.
            update host_event {
                accept [event_name, event_boost];
                validate present(event_name);
                validate numericality(event_boost, min: 1, max: 8);
            }

            /// Ends the event: clear the name and set the boost back to 1.
            update clear_event {
                accept [event_name, event_boost];
            }
        }
    }
}
