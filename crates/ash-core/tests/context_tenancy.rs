//! Context multitenancy keeps each tenant's rows of a resource apart in the data layer,
//! as Ash's does: a table per tenant in memory, as its ETS data layer keeps, while
//! SQLite, like AshSqlite, refuses rather than mix them.

use ash_core::{Context, DataLayer, Error, Filter, Resource, SchemaSupport, TransactionSupport};
use ash_memory::Memory;
use ash_sqlite::Sqlite;

pub use booking::Booking;
pub use port::Port;

pub mod port {
    use super::booking::Booking;
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        /// Shared by every tenant.
        Port {
            table "ctx_ports";

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
        /// Each tenant's bookings live apart from the others'.
        Booking {
            table "ctx_bookings";

            multitenancy {
                strategy: context;
            }

            attributes {
                id: Uuid [pk];
                port_id: Uuid;
                tonnes: i64;
            }

            relationships {
                belongs_to port: Port [fk: port_id];
            }

            actions {
                create create { primary; accept [port_id, tonnes]; }
                read read { primary; }
                update weigh { primary; accept [tonnes]; }
                destroy cancel { primary; }
            }
        }
    }
}

/// Every read and write of a context-tenant resource stays in its tenant: direct reads,
/// writes by id, aggregates and filters through a relationship, loads, and writes in a
/// transaction.
pub async fn tenants_stay_apart<D: DataLayer + TransactionSupport + Clone>(ctx: Context<D>, acme: &str, globex: &str) {
    let acme = ctx.with_tenant(acme);
    let globex = ctx.with_tenant(globex);
    let leo = Port::create(&ctx).code("LEO").await.unwrap();
    let ours = Booking::create(&acme).port_id(leo.id).tonnes(10).await.unwrap();
    Booking::create(&acme).port_id(leo.id).tonnes(30).await.unwrap();
    let theirs = Booking::create(&globex).port_id(leo.id).tonnes(5).await.unwrap();

    let tonnes = |ctx: Context<D>| async move {
        let mut tonnes: Vec<i64> = Booking::query(&ctx).all().await.unwrap().iter().map(|b| b.tonnes).collect();
        tonnes.sort();
        tonnes
    };
    assert_eq!(tonnes(acme.clone()).await, [10, 30]);
    assert_eq!(tonnes(globex.clone()).await, [5]);

    // Knowing another tenant's id reaches nothing.
    assert!(matches!(Booking::get(&globex, ours.id).await, Err(Error::NotFound)));
    let err = ash_core::update_dynamic(
        &globex,
        &Booking::DEF,
        Booking::DEF.action("weigh").unwrap(),
        ours.id,
        [("tonnes".to_string(), ash_core::Value::Int(99))].into_iter().collect(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::NotFound), "{err:?}");
    theirs.cancel_on(&globex).await.unwrap();
    assert_eq!(tonnes(globex.clone()).await, Vec::<i64>::new());
    assert_eq!(tonnes(acme.clone()).await, [10, 30]);

    // Reads through a relationship see the tenant's rows only.
    let port = Port::query(&acme)
        .filter(Filter::eq("id", leo.id))
        .load_aggregate(Port::booking_count)
        .load_aggregate(Port::booked_tonnes)
        .load_rel(Port::bookings)
        .one()
        .await
        .unwrap();
    assert_eq!((port.booking_count, port.booked_tonnes), (Some(2), Some(40)));
    assert_eq!(port.bookings.loaded().unwrap().len(), 2);
    let heavy = Port::query(&globex)
        .filter(Filter::eq("id", leo.id) & Filter::related("bookings", Filter::gt("tonnes", 20)))
        .all()
        .await
        .unwrap();
    assert!(heavy.is_empty());

    // Writes in a transaction land in the tenant too.
    let in_tx = acme
        .transaction(|tx| async move { Booking::create(&tx).port_id(leo.id).tonnes(1).await })
        .await
        .unwrap();
    assert!(Booking::get(&acme, in_tx.id).await.is_ok());
    assert!(matches!(Booking::get(&globex, in_tx.id).await, Err(Error::NotFound)));

    // A tenant-scoped resource needs a tenant.
    assert!(matches!(
        Booking::query(&ctx).all().await,
        Err(Error::TenantRequired { .. })
    ));
}

#[tokio::test]
async fn memory_keeps_tenants_apart() {
    tenants_stay_apart(Context::new(Memory::new()), "acme", "globex").await;
}

#[tokio::test]
async fn sqlite_refuses_context_tenancy() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install_resources(&[&Port::DEF, &Booking::DEF]).await.unwrap();
    let ctx = Context::new(sqlite);
    let leo = Port::create(&ctx).code("LEO").await.unwrap();
    let acme = ctx.with_tenant("acme");
    let err = Booking::create(&acme).port_id(leo.id).tonnes(10).await.unwrap_err();
    assert!(err.to_string().contains("no schemas"), "{err}");
    let err = Booking::query(&acme).all().await.unwrap_err();
    assert!(err.to_string().contains("no schemas"), "{err}");
}
