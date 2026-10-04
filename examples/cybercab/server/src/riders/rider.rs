use ash_core::resource;
use uuid::Uuid;

use crate::rides::trip::Trip;
use crate::types::RiderTier;

resource! {
    /// Someone who hails Cybercabs. Operators see a first name and a last initial, never
    /// a full name or a phone number.
    Rider {
        table "riders";
        timestamps;

        attributes {
            id: Uuid [pk];
            /// "Priya S."
            display_name: String;
            tier: RiderTier [enum];
            rating: f64;
            /// Last four digits, for confirming identity on an assist call.
            phone_last4: String;
            /// Needs extra time at the curb.
            assisted_boarding: bool [default: false];
        }

        relationships {
            has_many trips: Trip [fk: rider_id];
        }

        aggregates {
            trip_count: Option<i64> = count(trips, filter: status == "completed");
            lifetime_cents: Option<i64> = sum(trips, fare_cents, filter: status == "completed");
        }

        actions {
            create sign_up {
                primary;
                accept [display_name, tier, rating, phone_last4, assisted_boarding];
                validate numericality(rating, min: 1, max: 5);
            }

            read read {
                primary;
                pagination keyset: true, offset: true, countable: true, required: false;
            }
        }
    }
}
