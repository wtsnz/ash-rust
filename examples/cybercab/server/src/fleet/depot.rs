use ash_core::resource;
use uuid::Uuid;

use super::cab::Cab;

resource! {
    /// A Supercharger hub. Cabs come home here to charge and to be serviced.
    Depot {
        table "depots";

        attributes {
            id: Uuid [pk];
            code: String;
            name: String;
            lng: f64;
            lat: f64;
            stalls: i64;
        }

        identities {
            identity unique_code: [code];
        }

        relationships {
            has_many cabs: Cab [fk: depot_id];
        }

        aggregates {
            cab_count: Option<i64> = count(cabs);
            charging: Option<i64> = count(cabs, filter: status == "charging");
        }

        actions {
            create open {
                primary;
                accept [code, name, lng, lat, stalls];
            }

            read read {
                primary;
                pagination keyset: true, offset: true, countable: true, required: false;
            }
        }
    }
}
