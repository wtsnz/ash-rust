//! `change atomic_update(field, expr)`, as Ash's `atomic_update(:field, expr(...))`: the
//! field set to an expression over the record as stored, in the update's statement where
//! the data layer runs one, from the record read first where it doesn't. Concurrent
//! updates lose nothing.

use ash_core::{Context, DataLayer, Resource, resource};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Tally {
        table "atomic_tallies";

        attributes {
            id: Uuid [pk];
            name: String;
            views: i64 [default: 0];
        }

        actions {
            create create { primary; accept [name, views]; }
            read read { primary; }

            update view {
                change atomic_update(views, views + 1);
            }

            update shout {
                change atomic_update(name, upper(name));
            }

            update scale {
                argument by: i64;
                change atomic_update(views, views * arg(by) - 1);
            }

            update cap {
                change atomic_update(views, if_else(views >= 10, 10, views));
            }
        }
    }
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    let ctx = Context::new(data);
    let tally = Tally::create(&ctx).name("quiet").views(2).await.unwrap();
    let viewed = tally.view(&ctx).await.unwrap();
    assert_eq!(viewed.views, 3);
    let shouted = viewed.shout(&ctx).await.unwrap();
    assert_eq!(shouted.name, "QUIET");
    let scaled = shouted.scale(&ctx).by(5).await.unwrap();
    assert_eq!(scaled.views, 14);
    let capped = scaled.cap(&ctx).await.unwrap();
    assert_eq!(capped.views, 10);

    // From the record as stored, so concurrent updates each count, where the data layer
    // runs the update as one statement (one that reads the record first can't promise it).
    if !ctx.data.can_update_atomically(&Tally::DEF) {
        return;
    }
    let fresh = Tally::create(&ctx).name("busy").await.unwrap();
    let handles: Vec<_> = (0..10)
        .map(|_| {
            let (ctx, fresh) = (ctx.clone(), fresh.clone());
            tokio::spawn(async move { fresh.view(&ctx).await.unwrap() })
        })
        .collect();
    for handle in handles {
        handle.await.unwrap();
    }
    let counted = Tally::query(&ctx).filter(ash_core::Filter::eq("id", fresh.id)).one().await.unwrap();
    assert_eq!(counted.views, 10);
}

#[tokio::test]
async fn atomic_updates_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn atomic_updates_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Tally::DEF]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn atomic_updates_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&Tally::DEF]).await.unwrap();
    scenario(pg).await;
}
