use ash_core::{Context, Error, Resource, TransactionSupport, resource};
use ash_memory::Memory;
use uuid::Uuid;

resource! {
    ConcurrentUser {
        table "concurrent_users";

    attributes {
        id: Uuid [pk];
        name: String;
        email: String;
    }

    actions {
        create create {
            primary;
            accept [name, email];
        }

        read read {
            primary;
        }

        update update {
            primary;
            accept [name, email];
        }
    }
    }}

#[tokio::test]
async fn test_memory_transaction_does_not_discard_concurrent_writes() {
    let mem = Memory::new();
    let ctx = Context::new(mem.clone());

    // 1. Initial user
    let alice = ConcurrentUser::create(&ctx)
        .name("Alice")
        .email("alice@example.com")
        .await
        .unwrap();

    // 2. Start a transaction on `mem`
    let (tx_start_tx, tx_start_rx) = tokio::sync::oneshot::channel();
    let (write_done_tx, write_done_rx) = tokio::sync::oneshot::channel();

    let mem_clone = mem.clone();
    let tx_handle = tokio::spawn(async move {
        mem_clone
            .transaction(|tx_mem| {
                let tx_mem = tx_mem.clone();
                async move {
                    let tx_ctx = Context::new(tx_mem);
                    // In-transaction write: create Carol
                    let carol = ConcurrentUser::create(&tx_ctx)
                        .name("Carol")
                        .email("carol@example.com")
                        .await
                        .unwrap();

                    // Signal that in-transaction write is done
                    tx_start_tx.send(()).unwrap();

                    // Wait for concurrent write outside transaction to complete
                    write_done_rx.await.unwrap();

                    Ok(carol)
                }
            })
            .await
    });

    // Wait until tx has taken snapshot and inserted Carol into tx_mem
    tx_start_rx.await.unwrap();

    // Perform concurrent write to `mem` while transaction is in-flight!
    let bob = ConcurrentUser::create(&ctx)
        .name("Bob")
        .email("bob@example.com")
        .await
        .unwrap();

    // Signal tx to commit
    write_done_tx.send(()).unwrap();

    let carol = tx_handle.await.unwrap().expect("tx should commit successfully");

    // Invariant: ALL THREE users must exist in memory! Bob must NOT be discarded!
    let all = ConcurrentUser::query(&ctx).all().await.unwrap();
    assert_eq!(all.len(), 3, "must contain Alice, Bob, and Carol");

    let ids: Vec<Uuid> = all.iter().map(Resource::id).collect();
    assert!(ids.contains(&alice.id), "Alice must be present");
    assert!(ids.contains(&bob.id), "Bob must NOT have been discarded by tx commit");
    assert!(ids.contains(&carol.id), "Carol must be present from tx commit");
}

#[tokio::test]
async fn test_memory_transaction_detects_concurrent_write_conflict_on_same_record() {
    let mem = Memory::new();
    let ctx = Context::new(mem.clone());

    // 1. Initial user Dave
    let dave = ConcurrentUser::create(&ctx)
        .name("Dave Initial")
        .email("dave@example.com")
        .await
        .unwrap();

    let (tx_start_tx, tx_start_rx) = tokio::sync::oneshot::channel();
    let (write_done_tx, write_done_rx) = tokio::sync::oneshot::channel();

    let dave_id = dave.id;
    let mem_clone = mem.clone();
    let tx_handle = tokio::spawn(async move {
        mem_clone
            .transaction(|tx_mem| {
                let tx_mem = tx_mem.clone();
                async move {
                    let tx_ctx = Context::new(tx_mem);
                    // Update Dave in transaction
                    ConcurrentUser::update(&tx_ctx, dave_id)
                        .name("Dave from Tx")
                        .await
                        .unwrap();

                    tx_start_tx.send(()).unwrap();
                    write_done_rx.await.unwrap();
                    Ok(())
                }
            })
            .await
    });

    tx_start_rx.await.unwrap();

    // Concurrently update the same record Dave outside the transaction!
    ConcurrentUser::update(&ctx, dave.id)
        .name("Dave Concurrent")
        .await
        .unwrap();

    write_done_tx.send(()).unwrap();

    // Invariant: Tx commit MUST fail with conflict error on Dave!
    let tx_res = tx_handle.await.unwrap();
    assert!(
        matches!(tx_res, Err(Error::DataLayer(ref msg)) if msg.contains("conflict")),
        "expected concurrent write conflict error, got: {tx_res:?}"
    );

    // Dave's live record must remain "Dave Concurrent"
    let dave_live = ash_core::get::<ConcurrentUser, _>(&ctx, dave.id).await.unwrap();
    assert_eq!(dave_live.name, "Dave Concurrent");
}
