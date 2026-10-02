//! A read in a tenant sees only that tenant's rows of an attribute-tenant resource,
//! including the rows it reaches through a relationship: aggregates, relationship
//! filters and loads, in memory and SQL alike.

use ash_core::{Context, DataLayer, Filter, Resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

use booking::Booking;
use port::Port;

pub mod port {
    use super::booking::Booking;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        /// Ports are shared by every tenant.
        Port {
            table "ports";

            attributes {
                id: Uuid [pk];
                code: String;
            }

            relationships {
                has_many bookings: Booking [fk: port_id];
            }

            aggregates {
                booking_count: Option<i64> = count(bookings);
                booked_tonnes: Option<i64> = sum(bookings, tonnes);
            }

            actions {
                create create { primary; accept [code]; }
                read read { primary; }
            }
        }
    }
}

pub mod booking {
    use super::port::Port;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        /// Each shipping line's bookings are its own.
        Booking {
            table "bookings";

            multitenancy {
                strategy: attribute;
                attribute: line;
            }

            attributes {
                id: Uuid [pk];
                line: String;
                port_id: Uuid;
                tonnes: i64;
            }

            relationships {
                belongs_to port: Port [fk: port_id];
            }

            actions {
                create create { primary; accept [port_id, tonnes]; }
                read read { primary; }
            }
        }
    }
}

async fn reads_reach_only_the_tenant<D: DataLayer + Clone>(data: D) {
    let ctx = Context::new(data);
    let helios = ctx.with_tenant("helios");
    let red_dust = ctx.with_tenant("red_dust");
    let leo = Port::create(&ctx).code("LEO").await.unwrap();
    Booking::create(&helios).port_id(leo.id).tonnes(10).await.unwrap();
    Booking::create(&red_dust).port_id(leo.id).tonnes(30).await.unwrap();
    Booking::create(&red_dust).port_id(leo.id).tonnes(5).await.unwrap();

    let port = |ctx: Context<D>| async move {
        Port::query(&ctx)
            .load_aggregate(Port::booking_count)
            .load_aggregate(Port::booked_tonnes)
            .one()
            .await
            .unwrap()
    };
    let seen = port(helios.clone()).await;
    assert_eq!((seen.booking_count, seen.booked_tonnes), (Some(1), Some(10)));
    let seen = port(red_dust.clone()).await;
    assert_eq!((seen.booking_count, seen.booked_tonnes), (Some(2), Some(35)));

    // Filtering through the relationship sees the tenant's bookings only.
    let heavy = |ctx: Context<D>| async move {
        Port::query(&ctx)
            .filter(Filter::related("bookings", Filter::gt("tonnes", 20)))
            .all()
            .await
            .unwrap()
            .len()
    };
    assert_eq!(heavy(helios.clone()).await, 0);
    assert_eq!(heavy(red_dust.clone()).await, 1);

    // As do loads.
    let loaded = Port::query(&helios)
        .load_rel(Port::bookings)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.bookings.loaded().unwrap().len(), 1);
}

#[tokio::test]
async fn memory_reads_reach_only_the_tenant() {
    reads_reach_only_the_tenant(Memory::new()).await;
}

#[tokio::test]
async fn sqlite_reads_reach_only_the_tenant() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Port::DEF, &Booking::DEF]).await.unwrap();
    reads_reach_only_the_tenant(sqlite).await;
}
