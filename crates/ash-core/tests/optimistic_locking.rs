//! An optimistic lock is an action's, as Ash's `optimistic_lock(:version)` change is: the
//! action writes only if the record still holds the version it was read at, else
//! `StaleRecord`, and adds one to it. Actions without it leave the version alone. It
//! guards the write however the action runs: as one statement, after a before-action
//! hook, as a destroy, by id (read first), and in bulk (a stale record left out).

use ash_core::{BulkUpdateOptions, Context, Error, FieldMap, Resource, TransactionSupport, Value, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

fn no_op(_: &mut FieldMap) -> ash_core::Result<()> {
    Ok(())
}

resource! {
    BankAccount {
        table "bank_accounts";

        attributes {
            id: Uuid [pk];
            holder: String;
            balance: i64;
            version: i64 [default: 1];
        }

        actions {
            create open {
                primary;
                accept [holder, balance];
            }

            read read { primary; }

            update deposit {
                change optimistic_lock(version);
                accept [balance];
            }

            // Runs a hook before it writes, so it can't be one statement: the lock still
            // guards the write.
            update audited_deposit {
                change optimistic_lock(version);
                change before_action(no_op);
                accept [balance];
                require_atomic false;
            }

            // No lock: the version stays where it is.
            update rename {
                accept [holder];
            }

            destroy close {
                change optimistic_lock(version);
            }

            // Sets the version itself: the lock's increment is set last, over it, as Ash
            // sets it in a before-action hook, after every change.
            update reset_and_deposit {
                change set(version = 1);
                change optimistic_lock(version);
                change before_action(no_op);
                accept [balance];
                require_atomic false;
            }

            update reset_in_bulk {
                change set(version = 1);
                change optimistic_lock(version);
                accept [balance];
            }
        }
    }
}

pub mod drafts {
    use super::*;

    resource! {
        Draft {
            table "drafts";

            attributes {
                id: Uuid [pk];
                body: String;
                version: Option<i64>;
                revision: i64 [default: 1];
            }

            actions {
                create create { primary; accept [body]; }
                read read { primary; }

                update edit {
                    change optimistic_lock(version);
                    accept [body];
                }

                // Two locks: each is checked and each moves on, as two Ash `optimistic_lock`
                // changes would.
                update edit_both {
                    change optimistic_lock(version);
                    change optimistic_lock(revision);
                    accept [body];
                }

                update revise {
                    change optimistic_lock(revision);
                    accept [body];
                }
            }
        }
    }
}

use drafts::Draft;

async fn scenario<D: TransactionSupport + Clone + 'static>(ctx: Context<D>) {
    let account = BankAccount::open(&ctx).holder("Alice").balance(100).await.unwrap();
    assert_eq!(account.version, 1);
    let stale = account.clone();

    // A locked update adds one to the version; one from the version before is stale.
    let deposited = account.deposit_on(&ctx).balance(150).await.unwrap();
    assert_eq!((deposited.version, deposited.balance), (2, 150));
    match stale.deposit_on(&ctx).balance(200).await {
        Err(Error::StaleRecord { resource, id }) => {
            assert_eq!(resource, "BankAccount");
            assert_eq!(id, Value::from(stale.id));
        }
        other => panic!("expected StaleRecord, got {other:?}"),
    }

    // So is one that can't run as one statement.
    let stale = deposited.clone();
    let audited = deposited.audited_deposit_on(&ctx).balance(175).await.unwrap();
    assert_eq!(audited.version, 3);
    assert!(matches!(stale.audited_deposit_on(&ctx).balance(1).await, Err(Error::StaleRecord { .. })));

    // An unlocked action leaves the version alone, and isn't refused for it.
    let renamed = deposited.rename_on(&ctx).holder("Alicia").await.unwrap();
    assert_eq!(renamed.version, 3, "the version is the lock's alone");

    // By id, a locked update reads the record first and writes at the version it read.
    let action = BankAccount::DEF.action("deposit").unwrap();
    let by_id = ash_core::update_dynamic(&ctx, &BankAccount::DEF, action, account.id, FieldMap::from([("balance".to_string(), Value::from(300))]))
        .await
        .unwrap();
    assert_eq!(by_id.get("version"), Some(&Value::Int(4)));

    // In bulk, a record changed since it was read is left out, as Ash's bulk update
    // leaves it out.
    let current = BankAccount::get(&ctx, account.id).await.unwrap();
    let other = BankAccount::open(&ctx).holder("Bob").balance(10).await.unwrap();
    let mut balance = FieldMap::new();
    balance.insert("balance".into(), Value::from(1));
    let rows = vec![(stale.clone(), balance.clone()), (other.clone(), balance)];
    let result = ash_core::bulk_update::<BankAccount, D, _, _>(&ctx, "deposit", rows, BulkUpdateOptions::default()).await.unwrap();
    assert_eq!(result.count, 1, "the stale record is left out");
    assert_eq!(result.error_count, 0, "and isn't an error");
    assert_eq!(BankAccount::get(&ctx, other.id).await.unwrap().balance, 1);
    assert_eq!(BankAccount::get(&ctx, account.id).await.unwrap().balance, current.balance);

    // A locked destroy, too.
    match ash_core::destroy_existing::<BankAccount, D>(&ctx, "close", stale).await {
        Err(Error::StaleRecord { .. }) => {}
        other => panic!("expected StaleRecord, got {other:?}"),
    }
    ash_core::destroy_existing::<BankAccount, D>(&ctx, "close", current).await.unwrap();
    assert!(matches!(BankAccount::get(&ctx, account.id).await, Err(Error::NotFound)));
}

async fn the_increment_is_set_last<D: TransactionSupport + Clone + 'static>(ctx: Context<D>) {
    // One record at a time, after a hook.
    let account = BankAccount::open(&ctx).holder("Cy").balance(1).await.unwrap();
    let stale = account.clone();
    let reset = account.reset_and_deposit_on(&ctx).balance(2).await.unwrap();
    assert_eq!(reset.version, 2, "the lock's increment wins over the action's set");
    assert!(matches!(stale.reset_and_deposit_on(&ctx).balance(3).await, Err(Error::StaleRecord { .. })));

    // In bulk.
    let account = BankAccount::open(&ctx).holder("Di").balance(1).await.unwrap();
    let rows = vec![(account.clone(), FieldMap::from([("balance".to_string(), Value::from(2))]))];
    let result = ash_core::bulk_update::<BankAccount, D, _, _>(&ctx, "reset_in_bulk", rows.clone(), BulkUpdateOptions::default()).await.unwrap();
    assert_eq!(result.count, 1);
    assert_eq!(BankAccount::get(&ctx, account.id).await.unwrap().version, 2);
    let result = ash_core::bulk_update::<BankAccount, D, _, _>(&ctx, "reset_in_bulk", rows, BulkUpdateOptions::default()).await.unwrap();
    assert_eq!(result.count, 0, "the copy read at version 1 is stale");
}

async fn null_and_many_versions<D: TransactionSupport + Clone + 'static>(ctx: Context<D>) {
    // A null version is held as read: once it's moved on, a copy read null is stale.
    let draft = Draft::create(&ctx).body("a").await.unwrap();
    assert_eq!(draft.version, None);
    let stale = draft.clone();
    let edited = draft.edit_on(&ctx).body("b").await.unwrap();
    assert_eq!(edited.version, Some(1));
    assert!(matches!(stale.edit_on(&ctx).body("c").await, Err(Error::StaleRecord { .. })));

    // Every lock is checked, and every lock moves on.
    let stale = edited.clone();
    let revised = edited.revise_on(&ctx).body("d").await.unwrap();
    assert_eq!((revised.version, revised.revision), (Some(1), 2));
    assert!(matches!(stale.edit_both_on(&ctx).body("e").await, Err(Error::StaleRecord { .. })), "the second lock moved on");
    let both = revised.edit_both_on(&ctx).body("f").await.unwrap();
    assert_eq!((both.version, both.revision), (Some(2), 3));
}

#[tokio::test]
async fn optimistic_locks_in_memory() {
    scenario(Context::new(Memory::new())).await;
    the_increment_is_set_last(Context::new(Memory::new())).await;
    null_and_many_versions(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn optimistic_locks_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&BankAccount::DEF, &Draft::DEF]).await.unwrap();
    scenario(Context::new(sqlite.clone())).await;
    the_increment_is_set_last(Context::new(sqlite.clone())).await;
    null_and_many_versions(Context::new(sqlite)).await;
}
