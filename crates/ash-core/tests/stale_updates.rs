//! An update writes only the attributes it changes, as Ash's do, so one made from a stale
//! copy of a record doesn't write that copy's other fields back over newer values.

use ash_core::{Context, DataLayer, Resource, SchemaSupport, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Vehicle {
        table "vehicles";

        attributes {
            id: Uuid [pk];
            call_sign: String;
            status: String;
            speed_kph: i64;
        }

        actions {
            create commission {
                primary;
                accept [call_sign, status, speed_kph];
            }

            read read {
                primary;
            }

            /// Telemetry: how fast it's going.
            update report {
                primary;
                accept [speed_kph];
            }

            /// An operator's command.
            update recall {
                change set(status = "returning");
            }

            update set_status {
                accept [status];
            }
        }
    }
}

async fn neither_update_undoes_the_other<D: DataLayer>(ctx: Context<D>) {
    let cab = Vehicle::commission(&ctx)
        .call_sign("CC-0101".to_string())
        .status("available".to_string())
        .speed_kph(0)
        .await
        .unwrap();
    // The telemetry loop and an operator each hold the same copy.
    let id = cab.id;
    let telemetry = cab.clone();
    cab.recall_on(&ctx).await.unwrap();
    telemetry.report_on(&ctx).speed_kph(42).await.unwrap();

    let stored = Vehicle::get(&ctx, id).await.unwrap();
    assert_eq!((stored.status.as_str(), stored.speed_kph), ("returning", 42));
}

/// Setting a field to the value the copy already holds isn't a change, as in Ash: it
/// doesn't write the stale value back over a newer one.
async fn an_unchanged_value_is_not_written<D: DataLayer>(ctx: Context<D>) {
    let cab = Vehicle::commission(&ctx)
        .call_sign("CC-0102".to_string())
        .status("available".to_string())
        .speed_kph(0)
        .await
        .unwrap();
    let stale = cab.clone();
    cab.recall_on(&ctx).await.unwrap();
    stale
        .set_status_on(&ctx)
        .status("available".to_string())
        .await
        .unwrap();
    let stored = Vehicle::get(&ctx, stale.id).await.unwrap();
    assert_eq!(stored.status, "returning");
}

#[tokio::test]
async fn memory_updates_write_only_their_changes() {
    neither_update_undoes_the_other(Context::new(Memory::new())).await;
    an_unchanged_value_is_not_written(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn sqlite_updates_write_only_their_changes() {
    for check in 0..2 {
        let db = Sqlite::memory().await.unwrap();
        db.install_resources(&[&Vehicle::DEF]).await.unwrap();
        let ctx = Context::new(db);
        match check {
            0 => neither_update_undoes_the_other(ctx).await,
            _ => an_unchanged_value_is_not_written(ctx).await,
        }
    }
}
