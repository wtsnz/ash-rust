//! UTC datetimes are stored in UTC at their precision, as Ash's `:utc_datetime` and
//! `:utc_datetime_usec` are, so every data layer orders, compares and returns them alike.

use ash_core::{Context, DataLayer, Filter, Resource, UtcDateTime, UtcDateTimeUsec, Value, resource};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

resource! {
    Shift {
        table "shifts";

        attributes {
            id: Uuid [pk];
            label: String;
            starts_at: UtcDateTime;
            logged_at: Option<UtcDateTimeUsec>;
        }

        actions {
            create create { primary; accept [label, starts_at, logged_at]; }
            read read { primary; }
        }
    }
}

fn at(raw: &str) -> UtcDateTime {
    UtcDateTime::parse(raw).unwrap()
}

async fn datetimes_are_stored_in_utc<D: DataLayer>(data: D) {
    let ctx = Context::new(data);
    // 23:30 UTC, written with an offset that puts it on the next day.
    Shift::create(&ctx)
        .label("late")
        .starts_at(at("2187-02-12T01:30:00+02:00"))
        .logged_at(UtcDateTimeUsec::parse("2187-02-12T08:00:00.5+00:00").unwrap())
        .await
        .unwrap();
    // 23:45 UTC; the fraction is below `UtcDateTime`'s precision.
    Shift::create(&ctx)
        .label("early")
        .starts_at(at("2187-02-11T23:45:00.900Z"))
        .await
        .unwrap();

    let shifts = Shift::query(&ctx).sort(Shift::starts_at).all().await.unwrap();
    let labels: Vec<&str> = shifts.iter().map(|shift| shift.label.as_str()).collect();
    assert_eq!(labels, ["late", "early"]);
    assert_eq!(shifts[0].starts_at.as_str(), "2187-02-11T23:30:00Z");
    assert_eq!(shifts[1].starts_at.as_str(), "2187-02-11T23:45:00Z");
    assert_eq!(
        shifts[0].logged_at.as_ref().map(UtcDateTimeUsec::as_str),
        Some("2187-02-12T08:00:00.500000Z")
    );

    // Filter values are read as the attribute's type, whatever their offset.
    let matching = |filter: Filter| {
        let ctx = &ctx;
        async move {
            let mut labels: Vec<String> = Shift::query(ctx)
                .filter(filter)
                .all()
                .await
                .unwrap()
                .into_iter()
                .map(|shift| shift.label)
                .collect();
            labels.sort();
            labels
        }
    };
    let raw = |text: &str| Value::String(text.to_string());
    assert_eq!(matching(Filter::lt("starts_at", raw("2187-02-12T00:40:00+01:00"))).await, ["late"]);
    assert_eq!(matching(Filter::eq("starts_at", raw("2187-02-12T01:30:00+02:00"))).await, ["late"]);
    assert_eq!(matching(Filter::eq("starts_at", raw("2187-02-11T23:45:00Z"))).await, ["early"]);
}

#[tokio::test]
async fn memory_datetimes_are_stored_in_utc() {
    datetimes_are_stored_in_utc(Memory::new()).await;
}

#[tokio::test]
async fn sqlite_datetimes_are_stored_in_utc() {
    let sqlite = Sqlite::memory().await.unwrap();
    ash_core::SchemaSupport::install_resources(&sqlite, &[&Shift::DEF]).await.unwrap();
    datetimes_are_stored_in_utc(sqlite).await;
}
