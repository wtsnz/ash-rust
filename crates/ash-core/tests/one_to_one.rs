//! A one-to-one relationship declared on both sides: `has_one` on the parent and
//! `belongs_to` on the child. Loaded values are boxed, so the two structs can refer to
//! each other.

use ash_core::{Context, DataLayer, Resource, SchemaSupport};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

pub mod pilot {
    use super::license::License;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Pilot {
            table "pilots";
            attributes {
                id: Uuid [pk];
                name: String;
            }
            relationships {
                has_one license: License [fk: pilot_id];
            }
            actions {
                create create { primary; accept [name]; }
                read read { primary; }
            }
        }
    }
}

pub mod license {
    use super::pilot::Pilot;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        License {
            table "licenses";
            attributes {
                id: Uuid [pk];
                pilot_id: Uuid;
                grade: String;
            }
            relationships {
                belongs_to pilot: Pilot [fk: pilot_id];
            }
            actions {
                create create { primary; accept [pilot_id, grade]; }
                read read { primary; }
            }
        }
    }
}

use license::License;
use pilot::Pilot;

async fn scenario<D: DataLayer>(ctx: Context<D>) {
    let pilot = Pilot::create(&ctx).name("Yuri").await.unwrap();
    License::create(&ctx).pilot_id(pilot.id).grade("deep space").await.unwrap();

    let loaded = Pilot::query(&ctx).load_rel(Pilot::license).one().await.unwrap();
    let license = loaded.license.as_option().unwrap().expect("the pilot's license");
    assert_eq!(license.grade, "deep space");

    let loaded = License::query(&ctx).load_rel(License::pilot).one().await.unwrap();
    assert_eq!(loaded.pilot.as_option().unwrap().map(|p| p.name.as_str()), Some("Yuri"));
}

#[tokio::test]
async fn one_to_one_both_ways_in_memory() {
    scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn one_to_one_both_ways_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Pilot::DEF, &License::DEF]).await.unwrap();
    scenario(Context::new(sqlite)).await;
}
