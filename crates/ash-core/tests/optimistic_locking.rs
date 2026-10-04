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
        }
    }
}

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

#[tokio::test]
async fn optimistic_locks_in_memory() {
    scenario(Context::new(Memory::new())).await;
}

#[tokio::test]
async fn optimistic_locks_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&BankAccount::DEF]).await.unwrap();
    scenario(Context::new(sqlite)).await;
}
