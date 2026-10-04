//! A read's `prepare after_action(f)`, as Ash's: `f` runs on the records every read
//! through the action finds, with the read's arguments, and its failure fails the read.

use std::sync::atomic::{AtomicUsize, Ordering};

use ash_core::{Context, Error, FieldMap, Value, get_metadata, put_metadata, resource};
use ash_memory::Memory;
use uuid::Uuid;

static SEEN: AtomicUsize = AtomicUsize::new(0);

resource! {
    Book {
        table "read_after_action_books";

        attributes {
            id: Uuid [pk];
            title: String;
        }

        actions {
            read read { primary; }
            create create { primary; accept [title]; }

            read counted {
                argument refuse: bool [default: false];
                metadata seen: i64;
                prepare after_action(count_books);
            }
        }
    }
}

fn count_books(arguments: &FieldMap, records: &mut [FieldMap]) -> ash_core::Result<()> {
    if arguments.get("refuse") == Some(&Value::Bool(true)) {
        return Err(Error::Invalid("refused".into()));
    }
    for record in records.iter_mut() {
        let seen = SEEN.fetch_add(1, Ordering::SeqCst) as i64 + 1;
        put_metadata(record, "seen", seen);
        assert_eq!(get_metadata(record, "seen"), Some(&Value::Int(seen)));
    }
    Ok(())
}

#[tokio::test]
async fn a_read_runs_its_after_action_on_what_it_found() {
    let ctx = Context::new(Memory::new());
    Book::create(&ctx).title("one").await.unwrap();
    Book::create(&ctx).title("two").await.unwrap();

    let found = Book::query(&ctx).action("counted").all().await.unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(SEEN.load(Ordering::SeqCst), 2);
    // Not through another read.
    Book::query(&ctx).all().await.unwrap();
    assert_eq!(SEEN.load(Ordering::SeqCst), 2);

    let refused = Book::query(&ctx).action("counted").argument("refuse", true).all().await;
    assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
}
