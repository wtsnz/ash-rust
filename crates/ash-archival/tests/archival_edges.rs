//! Soft-destroy cases next to the basic scenario in `archival.rs`: locking, timestamps,
//! field policies, cycles, hard deletes of archival children, and writes that must not
//! reach archived records.

use std::sync::{Arc, Mutex};

use ash_archival::archival;
use ash_core::{
    Actor, BulkDestroyOptions, CompiledQuery, Context, DataLayer, Error, Multi, Notification,
    Resource, ResourceExt, SyncFnNotifier, Value, bulk_destroy, destroy_existing,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

pub mod ledger_mod {
    use super::*;

    #[archival]
    resource! {
        Ledger {
            table "ledgers";

            timestamps;

            attributes {
                id: Uuid [pk];
                name: String;
                version: i64 [version];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
use ledger_mod::Ledger;

pub mod payroll_mod {
    use super::*;

    #[archival]
    resource! {
        Payroll {
            table "payrolls";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                name: String;
                salary: Option<i64>;
            }

            field_policies {
                field salary {
                    authorize_if actor_attribute_equals(role, "admin");
                }
            }

            actions {
                create create { primary; accept [name, salary]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
use payroll_mod::Payroll;

pub mod node_mod {
    use super::*;

    /// Archiving a node archives its children, and the data may hold a cycle.
    #[archival]
    resource! {
        Node {
            table "nodes";

            attributes {
                id: Uuid [pk];
                parent_id: Option<Uuid>;
            }

            relationships {
                has_many children: Node [fk: parent_id];
            }

            archive {
                archive_related [children];
            }

            actions {
                create create { primary; accept [id, parent_id]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
use node_mod::Node;

pub mod cell_mod {
    use ash_core::resource;
    use uuid::Uuid;

    // A hard destroy that cascades through a cycle.
    resource! {
        Cell {
            table "cells";

            attributes {
                id: Uuid [pk];
                parent_id: Option<Uuid>;
            }

            relationships {
                has_many children: Cell [fk: parent_id];
            }

            actions {
                create create { primary; accept [id, parent_id]; }
                read read { primary; }
                destroy destroy { primary; cascade_destroy [children]; }
            }
        }
    }
}
use cell_mod::Cell;

pub mod board_mod {
    use super::*;

    #[archival]
    resource! {
        Board {
            table "boards";

            attributes {
                id: Uuid [pk];
                name: String;
            }

            relationships {
                has_many cards: super::card_mod::Card [fk: board_id, on_delete: cascade];
            }

            archive {
                exclude_destroy_actions [purge];
            }

            actions {
                create create { primary; accept [name]; }
                read read { primary; }
                destroy destroy { primary; }
                destroy purge;
            }
        }
    }
}
use board_mod::Board;

pub mod card_mod {
    use super::*;

    #[archival]
    resource! {
        Card {
            table "cards";

            attributes {
                id: Uuid [pk];
                board_id: Uuid;
            }

            actions {
                create create { primary; accept [board_id]; }
                read read { primary; }
                destroy destroy { primary; }
            }
        }
    }
}
use card_mod::Card;

async fn stored<D: DataLayer>(
    ctx: &Context<D>,
    resource: &ash_core::ResourceDef,
) -> Vec<ash_core::FieldMap> {
    ctx.data
        .run_query(resource, &CompiledQuery::default())
        .await
        .unwrap()
}

async fn sqlite() -> Sqlite {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite
        .install(&[
            &Ledger::DEF,
            &Payroll::DEF,
            &Node::DEF,
            &Cell::DEF,
            &Board::DEF,
            &Card::DEF,
        ])
        .await
        .unwrap();
    sqlite
}

async fn soft_destroy_bumps_version_and_updated_at<D: DataLayer>(ctx: Context<D>) {
    let ledger = Ledger::create(&ctx).name("Main").await.unwrap();
    // Timestamps have one-second resolution, so start from an old one.
    let mut old = ash_core::FieldMap::new();
    old.insert("updated_at".into(), Value::String("2000-01-01T00:00:00Z".into()));
    ctx.data.update(&Ledger::DEF, ledger.id, old).await.unwrap();

    ledger.destroy(&ctx).await.expect("a locked record can be archived");

    let after = stored(&ctx, &Ledger::DEF).await.remove(0);
    assert_eq!(after.get("version"), Some(&Value::Int(2)));
    assert_ne!(
        after.get("updated_at"),
        Some(&Value::String("2000-01-01T00:00:00Z".into()))
    );
    assert!(matches!(after.get("archived_at"), Some(Value::String(_))));
    assert_eq!(after.get("name"), Some(&Value::String("Main".into())));
}

async fn soft_destroy_keeps_fields_the_actor_cannot_read<D: DataLayer + Clone>(data: D) {
    let admin = Context::new(data.clone()).with_actor(Actor::new(Uuid::new_v4()).with_role("admin"));
    let member = Context::new(data).with_actor(Actor::new(Uuid::new_v4()).with_role("member"));
    let created = Payroll::create(&admin).name("Ada").salary(100).await.unwrap();

    let seen = Payroll::get(&member, created.id).await.unwrap();
    assert_eq!(seen.salary, None, "the member cannot read the salary");
    seen.destroy(&member).await.unwrap();

    let row = stored(&admin, &Payroll::DEF).await.remove(0);
    assert_eq!(row.get("salary"), Some(&Value::Int(100)));
    assert!(matches!(row.get("archived_at"), Some(Value::String(_))));
}

async fn cascades_stop_at_cycles<D: DataLayer>(ctx: Context<D>) {
    let (a, b) = (Uuid::from_u128(1), Uuid::from_u128(2));
    Node::create(&ctx).id(a).parent_id(b).await.unwrap();
    Node::create(&ctx).id(b).parent_id(a).await.unwrap();
    Node::get(&ctx, a).await.unwrap().destroy(&ctx).await.unwrap();
    assert_eq!(Node::query(&ctx).count().await.unwrap(), 0, "both nodes are archived");
    assert_eq!(stored(&ctx, &Node::DEF).await.len(), 2);

    Cell::create(&ctx).id(a).parent_id(b).await.unwrap();
    Cell::create(&ctx).id(b).parent_id(a).await.unwrap();
    Cell::get(&ctx, a).await.unwrap().destroy(&ctx).await.unwrap();
    assert!(stored(&ctx, &Cell::DEF).await.is_empty(), "both cells are deleted");
}

async fn hard_deletes_remove_archival_children<D: DataLayer>(ctx: Context<D>) {
    let board = Board::create(&ctx).name("Roadmap").await.unwrap();
    let archived = Card::create(&ctx).board_id(board.id).await.unwrap();
    Card::create(&ctx).board_id(board.id).await.unwrap();
    archived.destroy(&ctx).await.unwrap();
    assert_eq!(stored(&ctx, &Card::DEF).await.len(), 2);

    destroy_existing::<Board, D>(&ctx, "purge", board).await.unwrap();
    assert!(stored(&ctx, &Board::DEF).await.is_empty());
    assert!(
        stored(&ctx, &Card::DEF).await.is_empty(),
        "cards would point at a deleted board"
    );
}

async fn archived_records_are_out_of_reach<D: DataLayer>(ctx: Context<D>) {
    let ledger = Ledger::create(&ctx).name("Old").await.unwrap();
    let id = ledger.id;
    ledger.destroy(&ctx).await.unwrap();
    let archived_at = stored(&ctx, &Ledger::DEF).await.remove(0).remove("archived_at");

    let result = bulk_destroy::<Ledger, D>(&ctx, "destroy", &[id], BulkDestroyOptions::default())
        .await
        .unwrap();
    assert_eq!(result.count, 0, "an archived record is not archived again");
    assert_eq!(stored(&ctx, &Ledger::DEF).await.remove(0).remove("archived_at"), archived_at);
}

#[tokio::test]
async fn edges_in_memory() {
    soft_destroy_bumps_version_and_updated_at(Context::new(Memory::new())).await;
    soft_destroy_keeps_fields_the_actor_cannot_read(Memory::new()).await;
    cascades_stop_at_cycles(Context::new(Memory::new())).await;
    hard_deletes_remove_archival_children(Context::new(Memory::new())).await;
    archived_records_are_out_of_reach(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn edges_in_sqlite() {
    soft_destroy_bumps_version_and_updated_at(Context::new(sqlite().await)).await;
    soft_destroy_keeps_fields_the_actor_cannot_read(sqlite().await).await;
    cascades_stop_at_cycles(Context::new(sqlite().await)).await;
    hard_deletes_remove_archival_children(Context::new(sqlite().await)).await;
    archived_records_are_out_of_reach(Context::new(sqlite().await)).await;
}

#[tokio::test]
async fn bulk_notify_false_covers_cascaded_children() {
    let received = Arc::new(Mutex::new(Vec::<Notification>::new()));
    let sink = Arc::clone(&received);
    let notifier = Arc::new(SyncFnNotifier::new("sink", move |notification| {
        sink.lock().unwrap().push(notification.clone());
        Ok(())
    }));
    let ctx = Context::new(Memory::new()).with_notifier(notifier);
    let parent = Node::create(&ctx).id(Uuid::from_u128(1)).await.unwrap();
    Node::create(&ctx).id(Uuid::from_u128(2)).parent_id(parent.id).await.unwrap();
    received.lock().unwrap().clear();

    let options = BulkDestroyOptions {
        notify: false,
        ..BulkDestroyOptions::default()
    };
    let result = bulk_destroy::<Node, Memory>(&ctx, "destroy", &[parent.id], options)
        .await
        .unwrap();
    assert_eq!(result.count, 1);
    assert_eq!(Node::query(&ctx).count().await.unwrap(), 0, "the child is archived too");
    assert!(received.lock().unwrap().is_empty());
}

#[tokio::test]
async fn multi_destroy_returns_the_archived_record() {
    let ctx = Context::new(Memory::new());
    let ledger = Ledger::create(&ctx).name("Main").await.unwrap();
    let results = ctx
        .run_multi(Multi::new().destroy("archived", "destroy", ledger))
        .await
        .unwrap();
    let archived: &Ledger = results.get("archived").unwrap();
    assert!(archived.is_archived());
    assert_eq!(archived.version, 2);
}

#[tokio::test]
async fn graphql_cannot_destroy_an_archived_record_again() {
    use async_graphql::Request;

    let ctx = Context::new(Memory::new());
    let ledger = Ledger::create(&ctx).name("Main").await.unwrap();
    let id = ledger.id;
    let schema = ash_graphql::AshGraphQL::from_resources(&[&Ledger::DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let destroy = format!(
        r#"mutation {{ destroyLedger(input: {{ id: "{}" }}) {{ success errors {{ message }} }} }}"#,
        ledger.id
    );

    let first = schema.execute(Request::new(destroy.clone()).data(ctx.clone())).await;
    assert_eq!(first.data.into_json().unwrap()["destroyLedger"]["success"], true);
    let archived_at = stored(&ctx, &Ledger::DEF).await.remove(0).remove("archived_at");

    let again = schema.execute(Request::new(destroy).data(ctx.clone())).await;
    assert_eq!(again.data.into_json().unwrap()["destroyLedger"]["success"], false);
    assert_eq!(stored(&ctx, &Ledger::DEF).await.remove(0).remove("archived_at"), archived_at);

    // The Rust API agrees.
    assert!(matches!(Ledger::get(&ctx, id).await, Err(Error::NotFound)));
}
