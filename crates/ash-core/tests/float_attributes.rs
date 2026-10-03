//! A plain `f64` is a float attribute, as Ash's `:float` is a native float: typed setters,
//! filters, sorts and reads all use the number, on every data layer.

use ash_core::{Context, DataLayer, Error, Float, Resource, SchemaSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Beacon {
        table "beacons";

        attributes {
            id: Uuid [pk];
            name: String;
            lat: f64;
            lng: f64;
            accuracy_m: Option<f64>;
        }

        actions {
            create place {
                primary;
                accept [name, lat, lng, accuracy_m];
                validate numericality(lat, min: -90, max: 90);
            }
            update nudge { primary; accept [lat, lng]; }
            read read { primary; }
        }
    }
}

async fn floats_round_trip<D: DataLayer>(data: D) {
    let ctx = Context::new(data);
    let capitol = Beacon::place(&ctx).name("Capitol").lat(30.2747).lng(-97.7404).await.unwrap();
    assert_eq!((capitol.lat, capitol.lng, capitol.accuracy_m), (30.2747, -97.7404, None));
    Beacon::place(&ctx).name("Domain").lat(30.4021).lng(-97.7253).accuracy_m(4.5).await.unwrap();

    let north: Vec<String> = Beacon::query(&ctx)
        .filter(Beacon::lat.gt(30.3))
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.name)
        .collect();
    assert_eq!(north, ["Domain"]);
    let by_lat: Vec<f64> = Beacon::query(&ctx)
        .sort_desc(Beacon::lat)
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|b| b.lat)
        .collect();
    assert_eq!(by_lat, [30.4021, 30.2747]);

    let moved = capitol.nudge_on(&ctx).lat(30.275).lng(-97.74).await.unwrap();
    assert_eq!((moved.lat, moved.lng), (30.275, -97.74));
    let err = Beacon::place(&ctx).name("Nowhere").lat(f64::NAN).lng(0.0).await.unwrap_err();
    assert!(matches!(err, Error::Constraint { .. } | Error::Validation { .. }), "{err:?}");
    // Numericality checks floats as it does integers.
    let err = Beacon::place(&ctx).name("Off the map").lat(91.5).lng(0.0).await.unwrap_err();
    assert!(matches!(err, Error::Validation { ref field, .. } if field == "lat"), "{err:?}");
}

#[test]
fn float_reads_back_as_a_number() {
    assert_eq!(Float::parse("1.50").unwrap().value(), 1.5);
    assert_eq!(Float::try_from(-97.7404).unwrap().value(), -97.7404);
    assert!(Float::try_from(f64::INFINITY).is_err());
}

#[tokio::test]
async fn memory_float_attributes() {
    floats_round_trip(Memory::new()).await;
}

#[tokio::test]
async fn sqlite_float_attributes() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Beacon::DEF]).await.unwrap();
    floats_round_trip(sqlite).await;
}
