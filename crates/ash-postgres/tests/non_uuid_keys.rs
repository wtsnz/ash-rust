//! A primary key of any type, as Ash's are: an integer the data layer assigns
//! (`integer_primary_key`, a `bigserial` in Postgres), or text given with each record
//! (`attribute :code, :string, primary_key?: true`). Records are got, written, related
//! and paged by them, in every data layer.

use ash_core::{Context, Error, Filter, Resource, Value, resource};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod order {
    use super::*;

    resource! {
        Order {
            table "int_key_orders";

            attributes {
                id: i64 [pk];
                run: Uuid;
                label: String;
            }

            relationships {
                has_many lines: super::line::Line [fk: order_id];
            }

            aggregates {
                line_count: Option<i64> = count(lines);
            }

            actions {
                create create { primary; accept [run, label]; }
                read read { primary; }
                update relabel { primary; accept [label]; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod line {
    use super::*;

    resource! {
        Line {
            table "int_key_lines";

            attributes {
                id: Uuid [pk];
                order_id: i64;
                sku: String;
            }

            relationships {
                belongs_to order: super::order::Order [fk: order_id];
            }

            actions {
                create create { primary; accept [order_id, sku]; }
                read read { primary; }
            }
        }
    }
}

pub mod country {
    use super::*;

    resource! {
        Country {
            table "text_key_countries";

            attributes {
                code: String [pk];
                name: String;
            }

            actions {
                create create { primary; accept [code, name]; }
                read read { primary; }
                update rename { primary; accept [name]; }
                destroy destroy { primary; }
            }
        }
    }
}

pub mod tally {
    use super::*;

    resource! {
        /// Nothing but a key the data layer assigns.
        Tally {
            table "int_key_tallies";

            attributes {
                id: i64 [pk];
            }

            actions {
                create create { primary; }
                read read { primary; }
            }
        }
    }
}

use country::Country;
use line::Line;
use order::Order;
use tally::Tally;

async fn integer_keys<D: ash_core::TransactionSupport + Clone + 'static>(ctx: &Context<D>) {
    let run = Uuid::new_v4();
    let first = Order::create(ctx).run(run).label("first").await.unwrap();
    let second = Order::create(ctx).run(run).label("second").await.unwrap();
    // Assigned by the data layer, in order.
    assert!(first.id > 0, "{first:?}");
    assert!(second.id > first.id, "{first:?} {second:?}");
    assert_eq!(first.pk(), Value::Int(first.id));

    // Got, updated and destroyed by the key, given as the integer it is.
    assert_eq!(Order::get(ctx, first.id).await.unwrap().label, "first");
    let relabelled = Order::relabel(ctx, first.id).label("renamed").await.unwrap();
    assert_eq!((relabelled.id, relabelled.label.as_str()), (first.id, "renamed"));
    assert_eq!(Order::get(ctx, first.id).await.unwrap().label, "renamed");

    // Related by it: a line belongs to an order by its integer key.
    Line::create(ctx).order_id(second.id).sku("a").await.unwrap();
    Line::create(ctx).order_id(second.id).sku("b").await.unwrap();
    let counted = Order::query(ctx).filter(Filter::eq("id", second.id)).load_aggregate("line_count").one().await.unwrap();
    assert_eq!(counted.line_count, Some(2));
    let with_lines = Order::query(ctx)
        .filter(Filter::eq("run", run) & Filter::related("lines", Filter::eq("sku", "b")))
        .all()
        .await
        .unwrap();
    assert_eq!(with_lines.iter().map(|o| o.id).collect::<Vec<_>>(), [second.id]);

    // Paged by it, the key the keyset's tie-breaker.
    let page = Order::query(ctx).filter(Filter::eq("run", run)).page_keyset(1, None, None).await.unwrap();
    assert_eq!(page.results.iter().map(|o| o.id).collect::<Vec<_>>(), [first.id]);
    let next = Order::query(ctx).filter(Filter::eq("run", run)).page_keyset(1, page.after.as_deref(), None).await.unwrap();
    assert_eq!(next.results.iter().map(|o| o.id).collect::<Vec<_>>(), [second.id]);

    // Created in bulk, each assigned its own.
    let inputs = ["x", "y"].map(|label| [("run".to_string(), Value::Uuid(run)), ("label".to_string(), Value::from(label))]);
    let bulk = ash_core::bulk_create::<Order, D, _, _>(ctx, "create", inputs.map(ash_core::FieldMap::from), Default::default()).await.unwrap();
    let ids: Vec<i64> = bulk.records.iter().map(|o| o.id).collect();
    assert!(ids.len() == 2 && ids[0] > second.id && ids[1] > ids[0], "{ids:?}");

    Order::destroy(ctx, first.id).await.unwrap();
    assert!(matches!(Order::get(ctx, first.id).await, Err(Error::NotFound)));
}

async fn text_keys<D: ash_core::TransactionSupport + Clone + 'static>(ctx: &Context<D>) {
    // Unique to the run, as Postgres keeps earlier runs' rows.
    let code = format!("NZ-{}", Uuid::new_v4().simple());
    let nz = Country::create(ctx).code(code.clone()).name("New Zealand").await.unwrap();
    assert_eq!(nz.pk(), Value::String(code.clone()));

    assert_eq!(Country::get(ctx, code.as_str()).await.unwrap().name, "New Zealand");
    let renamed = Country::rename(ctx, code.as_str()).name("Aotearoa").await.unwrap();
    assert_eq!(renamed.code, code);
    assert_eq!(Country::get(ctx, code.clone()).await.unwrap().name, "Aotearoa");

    // The key is given with each record: a second with it is refused, and one without
    // it is missing it.
    assert!(Country::create(ctx).code(code.clone()).name("again").await.is_err());
    let missing = ash_core::create_dynamic(ctx, &Country::DEF, Country::DEF.action("create").unwrap(), [("name".to_string(), Value::from("Nowhere"))].into()).await;
    assert!(missing.is_err(), "{missing:?}");

    Country::destroy(ctx, code.as_str()).await.unwrap();
    assert!(matches!(Country::get(ctx, code).await, Err(Error::NotFound)));
}

async fn scenario<D: ash_core::TransactionSupport + Clone + 'static>(data: D) {
    let ctx = Context::new(data);
    integer_keys(&ctx).await;
    text_keys(&ctx).await;
    // A record of nothing but its assigned key.
    let (first, second) = (Tally::create(&ctx).await.unwrap(), Tally::create(&ctx).await.unwrap());
    assert!(second.id > first.id, "{first:?} {second:?}");
}

#[tokio::test]
async fn non_uuid_keys_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn non_uuid_keys_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Order::DEF, &Line::DEF, &Country::DEF, &Tally::DEF]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn non_uuid_keys_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&Order::DEF, &Line::DEF, &Country::DEF, &Tally::DEF]).await.unwrap();
    scenario(pg).await;
}
