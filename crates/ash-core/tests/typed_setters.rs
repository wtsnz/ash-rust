//! Optional attributes of every value type take a bare value, and text-backed types
//! work in `concat(...)` calculations.

use ash_core::{CiString, Context, DataLayer, Decimal, Inet, Resource, SchemaSupport, UtcDateTime};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

pub mod dock {
    use ash_core::{CiString, Decimal, Inet, UtcDateTime, resource};
    use uuid::Uuid;

    resource! {
        Dock {
            table "docks";
            attributes {
                id: Uuid [pk];
                code: CiString;
                relay: Inet;
                opened_at: Option<UtcDateTime>;
                fee: Option<Decimal>;
            }
            calculations {
                label: Option<String> = concat(code, " @ ", relay);
            }
            actions {
                create open { primary; accept [code, relay, opened_at, fee]; }
                read read { primary; }
            }
        }
    }
}

use dock::Dock;

async fn scenario<D: DataLayer>(ctx: Context<D>) {
    let opened_at = UtcDateTime::parse("2187-02-01T06:00:00Z").unwrap();
    let dock = Dock::open(&ctx)
        .code(CiString::parse("PZZ-A1").unwrap())
        .relay(Inet::parse("10.7.0.1").unwrap())
        .opened_at(opened_at.clone())
        .fee(Decimal::parse("12.50").unwrap())
        .await
        .unwrap();
    assert_eq!(dock.opened_at, Some(opened_at));
    assert_eq!(dock.code.to_string(), "PZZ-A1");

    let loaded = Dock::query(&ctx).calc(Dock::label).one().await.unwrap();
    assert_eq!(loaded.label.as_deref(), Some("PZZ-A1 @ 10.7.0.1"));
}

#[tokio::test]
async fn typed_setters_and_text_calculations_in_memory() {
    scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn typed_setters_and_text_calculations_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Dock::DEF]).await.unwrap();
    scenario(Context::new(sqlite)).await;
}
