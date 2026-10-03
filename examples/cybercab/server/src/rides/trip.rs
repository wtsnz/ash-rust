use ash_core::UtcDateTimeUsec;
use ash_state_machine::state_machine;
use uuid::Uuid;

use super::zone::ServiceZone;
use crate::fleet::cab::Cab;
use crate::riders::rider::Rider;

#[state_machine]
resource! {
    /// One ride, from the request to the drop-off. Its three legs are the journey:
    /// the cab's approach to the pickup, the wait at the curb, and the ride itself.
    /// Both legs' routes travel with the trip as encoded polylines.
    Trip {
        table "trips";
        timestamps;

        attributes {
            id: Uuid [pk];
            /// Shown to riders and support: "R-48213".
            code: String;
            rider_id: Uuid;
            zone_id: Uuid;
            cab_id: Option<Uuid>;
            pickup_name: String;
            pickup_lng: f64;
            pickup_lat: f64;
            dropoff_name: String;
            dropoff_lng: f64;
            dropoff_lat: f64;
            /// The ride leg, pickup to drop-off.
            ride_polyline: String;
            /// The approach leg, from wherever the cab was when it was assigned.
            approach_polyline: Option<String>;
            distance_m: i64;
            duration_s: i64;
            surge: f64;
            fare_cents: i64;
            requested_at: UtcDateTimeUsec;
            assigned_at: Option<UtcDateTimeUsec>;
            pickup_eta_at: Option<UtcDateTimeUsec>;
            arrived_at: Option<UtcDateTimeUsec>;
            picked_up_at: Option<UtcDateTimeUsec>;
            dropoff_eta_at: Option<UtcDateTimeUsec>;
            completed_at: Option<UtcDateTimeUsec>;
            cancelled_at: Option<UtcDateTimeUsec>;
            cancel_reason: Option<String>;
            /// The rider's rating of the ride, out of five.
            rating: Option<i64>;
        }

        identities {
            identity unique_code: [code];
        }

        indexes {
            index by_requested: [requested_at];
            // A cab's trips, newest first: a cab's latest trips page from here.
            index by_cab_requested: [cab_id, requested_at];
            index active_by_cab: [cab_id], where: "status IN ('assigned', 'arrived', 'riding')";
        }

        state_machine {
            state_attribute status;
            initial: "requested";
            transition assign, from: ["requested"], to: "assigned";
            transition arrive, from: ["assigned"], to: "arrived";
            transition board, from: ["arrived"], to: "riding";
            transition complete, from: ["riding"], to: "completed";
            transition cancel, from: ["requested", "assigned", "arrived"], to: "cancelled";
        }

        relationships {
            belongs_to rider: Rider [fk: rider_id];
            belongs_to cab: Cab [fk: cab_id];
            belongs_to zone: ServiceZone [fk: zone_id];
        }

        calculations {
            route_label: Option<String> = concat(pickup_name, " → ", dropoff_name);
        }

        actions {
            create request {
                primary;
                accept [code, rider_id, zone_id, pickup_name, pickup_lng, pickup_lat, dropoff_name, dropoff_lng, dropoff_lat, ride_polyline, distance_m, duration_s, surge, fare_cents, requested_at];
                validate numericality(fare_cents, min: 0);
            }

            read read {
                primary;
            }

            read recent {
                prepare sort(requested_at, desc);
                prepare limit(50);
            }

            update assign {
                accept [cab_id, approach_polyline, assigned_at, pickup_eta_at];
            }

            update arrive {
                accept [arrived_at];
            }

            update board {
                accept [picked_up_at, dropoff_eta_at];
            }

            update complete {
                accept [completed_at, rating];
                validate numericality(rating, min: 1, max: 5);
            }

            /// Called off: by the rider, or by an operator.
            update cancel {
                accept [cancelled_at, cancel_reason];
            }

            destroy archive {
                primary;
            }
        }
    }
}
