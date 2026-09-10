use ash_core::{Context, Error, Resource, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    resource BankAccount;
    table "bank_accounts";

    attributes {
        id: Uuid [pk],
        holder: String,
        balance: i64,
        version: i64 [version],
    }

    actions {
        create open {
            primary;
            accept [holder, balance];
        }

        update deposit {
            accept [balance];
        }
    }
}

#[tokio::test]
async fn test_optimistic_locking_memory_increments_and_detects_conflict() {
    let ctx = Context::new(Memory::new());

    // 1. Create record: initial version is 1
    let account = BankAccount::open(&ctx)
        .holder("Alice")
        .balance(100)
        .await
        .expect("account created");

    assert_eq!(account.version, 1);
    assert_eq!(account.balance, 100);

    // Take snapshot of current record (version 1)
    let stale_account = account.clone();

    // 2. First update: bumps version to 2
    let updated = account
        .deposit_on(&ctx)
        .balance(150)
        .await
        .expect("first deposit succeeds");

    assert_eq!(updated.version, 2);
    assert_eq!(updated.balance, 150);

    // 3. Second update from stale snapshot (version 1): should fail with StaleRecord
    let err = stale_account
        .deposit_on(&ctx)
        .balance(200)
        .await
        .expect_err("stale update must be rejected");

    match err {
        Error::StaleRecord { resource, id } => {
            assert_eq!(resource, "BankAccount");
            assert_eq!(id, stale_account.id);
        }
        other => panic!("expected Error::StaleRecord, got {other:?}"),
    }

    // 4. Update from latest record (version 2): bumps version to 3
    let updated_again = updated
        .deposit_on(&ctx)
        .balance(250)
        .await
        .expect("deposit on fresh record succeeds");

    assert_eq!(updated_again.version, 3);
    assert_eq!(updated_again.balance, 250);
}

#[tokio::test]
async fn test_optimistic_locking_sqlite_increments_and_detects_conflict() {
    let sqlite = Sqlite::memory().await.expect("sqlite in memory");
    sqlite
        .install(&[&BankAccount::DEF])
        .await
        .expect("create table");

    let ctx = Context::new(sqlite);

    // 1. Create record in SQLite: version is 1
    let account = BankAccount::open(&ctx)
        .holder("Bob")
        .balance(500)
        .await
        .expect("account created in sqlite");

    assert_eq!(account.version, 1);
    assert_eq!(account.balance, 500);

    let stale_account = account.clone();

    // 2. Update record: version bumps to 2
    let updated = account
        .deposit_on(&ctx)
        .balance(700)
        .await
        .expect("deposit succeeds in sqlite");

    assert_eq!(updated.version, 2);
    assert_eq!(updated.balance, 700);

    // 3. Stale update in SQLite: WHERE version = 1 affects 0 rows, returning StaleRecord
    let err = stale_account
        .deposit_on(&ctx)
        .balance(600)
        .await
        .expect_err("stale update in sqlite must fail");

    match err {
        Error::StaleRecord { resource, id } => {
            assert_eq!(resource, "BankAccount");
            assert_eq!(id, stale_account.id);
        }
        other => panic!("expected Error::StaleRecord, got {other:?}"),
    }

    // 4. Update from latest version in SQLite: succeeds and bumps to 3
    let updated_again = updated
        .deposit_on(&ctx)
        .balance(900)
        .await
        .expect("fresh deposit in sqlite succeeds");

    assert_eq!(updated_again.version, 3);
    assert_eq!(updated_again.balance, 900);
}
