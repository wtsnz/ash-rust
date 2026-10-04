use ash_core::UtcDateTimeUsec;
use ash_core::resource;
use uuid::Uuid;

resource! {
    /// The whole operation at one moment, taken every few seconds: the fleet by status,
    /// demand, and the day's trips and takings. The control room's sparklines are the
    /// last few minutes of these.
    PulseSample {
        table "pulse_samples";

        attributes {
            id: Uuid [pk];
            recorded_at: UtcDateTimeUsec;
            available: i64;
            dispatched: i64;
            on_trip: i64;
            returning: i64;
            charging: i64;
            maintenance: i64;
            /// Riders waiting for a cab.
            waiting: i64;
            completed_today: i64;
            revenue_cents_today: i64;
            /// Mean seconds from request to pickup, over the last trips.
            avg_wait_s: i64;
            /// Share of the fleet carrying or fetching a rider.
            utilization_pct: i64;
            avg_battery_pct: i64;
        }

        indexes {
            index by_time: [recorded_at];
        }

        actions {
            create record {
                primary;
                accept [recorded_at, available, dispatched, on_trip, returning, charging, maintenance, waiting, completed_today, revenue_cents_today, avg_wait_s, utilization_pct, avg_battery_pct];
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
