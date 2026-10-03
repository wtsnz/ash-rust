//! A bulk action writes each batch in a transaction, as Ash does by default
//! (`transaction: :batch`): a row that fails once its batch is written rolls the batch
//! back, and nothing in it is notified. `BulkTransaction::All` makes the whole action one
//! transaction and `Off` none. A data layer that can't transact, as Ash's ETS layer
//! can't, writes every row alone.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ash_core::{
    BulkTransaction, BulkUpdateOptions, Context, Error, FieldMap, Resource, Result, SchemaSupport,
    SyncFnNotifier, TransactionSupport, Value, resource,
};
use uuid::Uuid;
use ash_memory::Memory;
use ash_sqlite::Sqlite;

/// A reading below zero is written, then refused by the action's after-action hook.
fn refuse_negative(fields: &mut FieldMap) -> Result<()> {
    match fields.get("value") {
        Some(Value::Int(value)) if *value < 0 => Err(Error::Invalid("a reading below zero".into())),
        _ => Ok(()),
    }
}

resource! {
    Reading {
        table "readings";

        attributes {
            id: Uuid [pk];
            sensor: String;
            value: i64;
        }

        actions {
            create create {
                primary;
                accept [sensor, value];
            }

            read read {
                primary;
            }

            update record {
                accept [value];
                after_action refuse_negative;
            }
        }
    }
}

/// Three sensors reading zero, and a context that counts the updates it's notified of.
async fn sensors<D: TransactionSupport + SchemaSupport + 'static>(data: D) -> (Context<D>, Vec<Reading>, Arc<AtomicUsize>) {
    let notified = Arc::new(AtomicUsize::new(0));
    let heard = notified.clone();
    let ctx = Context::new(data).with_notifier(Arc::new(SyncFnNotifier::new("updates", move |notification| {
        if notification.action_kind == ash_core::ActionKind::Update {
            heard.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    })));
    ctx.install(&[&Reading::DEF]).await.unwrap();
    let mut readings = Vec::new();
    for sensor in ["a", "b", "c"] {
        readings.push(Reading::create(&ctx).sensor(sensor.to_string()).value(0).await.unwrap());
    }
    (ctx, readings, notified)
}

/// Records 10, -1 and 30, the middle one refused once written.
async fn record<D: TransactionSupport + 'static>(
    ctx: &Context<D>,
    readings: &[Reading],
    opts: BulkUpdateOptions,
) -> (usize, usize, Vec<i64>) {
    let inputs = readings.iter().cloned().zip([10, -1, 30]).map(|(reading, value)| {
        let mut input = FieldMap::new();
        input.insert("value".into(), Value::from(value));
        (reading, input)
    });
    let result = Reading::bulk_update_with_opts(ctx, "record", inputs, opts.stop_on_error(false))
        .await
        .unwrap();
    let mut stored: Vec<(String, i64)> =
        Reading::query(ctx).load().await.unwrap().into_iter().map(|r| (r.sensor, r.value)).collect();
    stored.sort();
    (result.count, result.error_count, stored.into_iter().map(|(_, value)| value).collect())
}

#[tokio::test]
async fn a_failed_row_rolls_back_its_batch() {
    let (ctx, readings, notified) = sensors(Sqlite::memory().await.unwrap()).await;
    assert_eq!(record(&ctx, &readings, BulkUpdateOptions::new()).await, (0, 3, vec![0, 0, 0]));
    assert_eq!(notified.load(Ordering::SeqCst), 0, "a rolled-back batch notifies nothing");
}

#[tokio::test]
async fn each_batch_is_its_own_transaction() {
    let (ctx, readings, notified) = sensors(Sqlite::memory().await.unwrap()).await;
    let opts = BulkUpdateOptions::new().batch_size(1);
    assert_eq!(record(&ctx, &readings, opts).await, (2, 1, vec![10, 0, 30]));
    assert_eq!(notified.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn the_whole_action_can_be_one_transaction() {
    let (ctx, readings, notified) = sensors(Sqlite::memory().await.unwrap()).await;
    let opts = BulkUpdateOptions::new().batch_size(1).transaction(BulkTransaction::All);
    assert_eq!(record(&ctx, &readings, opts).await, (0, 3, vec![0, 0, 0]));
    assert_eq!(notified.load(Ordering::SeqCst), 0);
}

/// Without a transaction there's nothing to roll back: the refused row was written and
/// fails alone.
#[tokio::test]
async fn without_a_transaction_a_row_fails_alone() {
    let (ctx, readings, notified) = sensors(Sqlite::memory().await.unwrap()).await;
    let opts = BulkUpdateOptions::new().transaction(BulkTransaction::Off);
    assert_eq!(record(&ctx, &readings, opts).await, (2, 1, vec![10, -1, 30]));
    assert_eq!(notified.load(Ordering::SeqCst), 2);

    // ash-memory can't transact, as Ash's ETS layer can't, so it writes rows alone too.
    let (ctx, readings, notified) = sensors(Memory::new()).await;
    assert_eq!(record(&ctx, &readings, BulkUpdateOptions::new()).await, (2, 1, vec![10, -1, 30]));
    assert_eq!(notified.load(Ordering::SeqCst), 2);
}
