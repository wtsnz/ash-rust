use ash_core::UtcDateTimeUsec;
use ash_core::resource;
use uuid::Uuid;

use crate::fleet::cab::Cab;

resource! {
    /// A cab's position and state at one moment. The last few minutes of samples are
    /// the trail drawn behind a cab on the map; older ones are pruned.
    TelemetrySample {
        table "telemetry_samples";

        attributes {
            id: Uuid [pk];
            cab_id: Uuid;
            recorded_at: UtcDateTimeUsec;
            lng: f64;
            lat: f64;
            speed_kph: i64;
            battery_pct: i64;
        }

        indexes {
            index by_cab_time: [cab_id, recorded_at];
        }

        relationships {
            belongs_to cab: Cab [fk: cab_id];
        }

        actions {
            create record {
                primary;
                accept [cab_id, recorded_at, lng, lat, speed_kph, battery_pct];
            }

            read read {
                primary;
                pagination keyset: true, offset: true, countable: true, required: false;
            }

            destroy prune {
                primary;
            }
        }
    }
}
