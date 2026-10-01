use ash_core::{CompiledQuery, DataLayer, Filter, Resource, Result, Value};

use crate::fixtures::searchable_notes::SearchableNote;
use crate::support::{Db, TestDb, on_every_backend};

const NOTES: &[(&str, &str, &str)] = &[
    ("00000000-0000-0000-0000-0000000000f1", "Printer on fire", "Ada@Example.com"),
    ("00000000-0000-0000-0000-0000000000f2", "50% off_sale", "grace@example.com"),
    ("00000000-0000-0000-0000-0000000000f3", r"C:\path*[x]?", "linus@example.org"),
];

async fn seed_notes(db: &TestDb) {
    db.install(&[&SearchableNote::DEF]).await.unwrap();
    for (id, title, email) in NOTES {
        db.exec(&format!(
            "INSERT INTO searchable_notes (id, title, email) VALUES ('{id}', '{title}', '{email}')"
        ))
        .await
        .unwrap();
    }
}

async fn run(db: &TestDb, filter: Filter) -> Result<Vec<String>> {
    let query = CompiledQuery {
        filter: Some(filter),
        ..Default::default()
    };
    let rows = match &db.db {
        Db::Sqlite(sqlite) => sqlite.run_query(&SearchableNote::DEF, &query).await?,
        Db::Postgres(pg) => pg.run_query(&SearchableNote::DEF, &query).await?,
    };
    let mut titles: Vec<String> = rows
        .iter()
        .map(|row| match row.get("title") {
            Some(Value::String(title)) => title.clone(),
            other => panic!("unexpected title {other:?}"),
        })
        .collect();
    titles.sort();
    Ok(titles)
}

async fn titles(db: &TestDb, filter: Filter) -> Vec<String> {
    run(db, filter).await.unwrap()
}

async fn text_filters_match_literally_and_respect_case(db: TestDb) {
    seed_notes(&db).await;
    let fire = vec!["Printer on fire".to_string()];
    let sale = vec!["50% off_sale".to_string()];
    let path = vec![r"C:\path*[x]?".to_string()];

    assert_eq!(titles(&db, Filter::contains("title", "on fi")).await, fire);
    assert_eq!(titles(&db, Filter::starts_with("title", "Printer")).await, fire);
    assert_eq!(titles(&db, Filter::ends_with("title", "fire")).await, fire);
    assert_eq!(titles(&db, Filter::starts_with("title", "fire")).await, Vec::<String>::new());
    assert_eq!(titles(&db, Filter::contains("title", "")).await.len(), 3);
    assert_eq!(
        titles(&db, !Filter::contains("title", "fire")).await,
        [sale.clone(), path.clone()].concat()
    );

    // String fields are case-sensitive.
    assert_eq!(titles(&db, Filter::contains("title", "ON FI")).await, Vec::<String>::new());
    assert_eq!(titles(&db, Filter::starts_with("title", "printer")).await, Vec::<String>::new());

    // Wildcards in the needle match only themselves.
    assert_eq!(titles(&db, Filter::contains("title", "%")).await, sale);
    assert_eq!(titles(&db, Filter::contains("title", "_")).await, sale);
    assert_eq!(titles(&db, Filter::starts_with("title", "50%")).await, sale);
    assert_eq!(titles(&db, Filter::contains("title", r"\")).await, path);
    assert_eq!(titles(&db, Filter::contains("title", "*")).await, path);
    assert_eq!(titles(&db, Filter::contains("title", "?")).await, path);
    assert_eq!(titles(&db, Filter::contains("title", "[x]")).await, path);
    assert_eq!(titles(&db, Filter::ends_with("title", "]?")).await, path);

    // CiString fields ignore case.
    assert_eq!(
        titles(&db, Filter::contains("email", "EXAMPLE.COM")).await,
        [sale.clone(), fire.clone()].concat()
    );
    assert_eq!(titles(&db, Filter::starts_with("email", "ada@")).await, fire);
    assert_eq!(titles(&db, Filter::ends_with("email", ".ORG")).await, path);

    // Typed field keys and DSL preparations build the same filters.
    assert_eq!(titles(&db, SearchableNote::title.contains("on fi")).await, fire);
    assert_eq!(titles(&db, SearchableNote::email.ends_with(".ORG")).await, path);
    let action = SearchableNote::DEF.action("titled_on_fire").unwrap();
    let mut prepared = None;
    for preparation in action.preparations {
        if let ash_core::PreparationDef::Filter(build) = preparation {
            prepared = Some(build());
        }
    }
    assert_eq!(titles(&db, prepared.expect("filter preparation")).await, fire);
}
on_every_backend!(text_filters_match_literally_and_respect_case);

async fn text_filters_reject_non_text_fields(db: TestDb) {
    seed_notes(&db).await;
    let err = run(&db, Filter::contains("id", "0000")).await.unwrap_err();
    assert!(
        err.to_string().contains("text filters need a string field"),
        "unexpected error: {err}"
    );
}
on_every_backend!(text_filters_reject_non_text_fields);
