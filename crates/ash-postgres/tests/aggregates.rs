//! Aggregates as each data layer loads them: Postgres laterally joins each relationship's
//! aggregates, SQLite groups them, as ash_sql's `:lateral` and `:grouped` strategies do,
//! and memory computes them as Ash's ETS layer does. One scenario, the same answers.

use ash_core::{
    ActionDef, ActionKind, Actor, AggregateDef, AggregateFilter, AttrType, AttributeDef, Check, CompiledQuery,
    ConstValue, DataLayer, FieldMap, Filter, PolicyDef, PolicyEffect, PolicyWhen, PreparationDef, RelationshipDef,
    ResourceDef, Sort, Value,
};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

const BASE: ResourceDef = ResourceDef {
    name: "",
    table: "",
    attributes: &[],
    relationships: &[],
    actions: &[],
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Postgres,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static LIBRARY: ResourceDef = ResourceDef {
    name: "AggLibrary",
    table: "agg_libraries",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    relationships: &[
        RelationshipDef::has_many("books", || &BOOK, "library_id"),
        RelationshipDef::many_to_many("readers", || &READER, || &MEMBERSHIP, "library_id", "reader_id"),
    ],
    actions: &[ActionDef::read("read").primary()],
    aggregates: &[
        AggregateDef::count("book_count", "books"),
        AggregateDef::count_where("long_count", "books", AggregateFilter::Eq("kind", ConstValue::Str("long"))),
        AggregateDef::sum("pages", "books", "pages"),
        AggregateDef::exists("has_books", "books"),
        AggregateDef::first("a_title", "books", "title", AttrType::String),
        AggregateDef::count("reader_count", "readers"),
    ],
    ..BASE
};

fn not_archived() -> Filter {
    Filter::eq("archived", false)
}

/// Archived books are out of the primary read's reach, so no aggregate counts them.
static BOOK: ResourceDef = ResourceDef {
    name: "AggBook",
    table: "agg_books",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("library_id", AttrType::Uuid),
        AttributeDef::required("title", AttrType::String),
        AttributeDef::required("kind", AttrType::String),
        AttributeDef::required("pages", AttrType::Integer),
        AttributeDef::required("archived", AttrType::Boolean),
    ],
    actions: &[ActionDef::read("read").primary().preparations(&[PreparationDef::Filter(not_archived)])],
    ..BASE
};

static READER: ResourceDef = ResourceDef {
    name: "AggReader",
    table: "agg_readers",
    attributes: &[AttributeDef::uuid_pk("id")],
    actions: &[ActionDef::read("read").primary()],
    ..BASE
};

/// Each membership is its reader's to read, so a reader count counts only the reader
/// reading it, as Ash authorizes an aggregate's query.
static MEMBERSHIP: ResourceDef = ResourceDef {
    name: "AggMembership",
    table: "agg_memberships",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("library_id", AttrType::Uuid),
        AttributeDef::required("reader_id", AttrType::Uuid),
    ],
    actions: &[ActionDef::read("read").primary()],
    policies: &[PolicyDef::when(
        PolicyWhen::ActionType(ActionKind::Read),
        &[PolicyEffect::AuthorizeIf(Check::RelatesToActor { field: "reader_id" })],
    )],
    ..BASE
};

const ALL: [&str; 6] = ["book_count", "long_count", "pages", "has_books", "a_title", "reader_count"];

async fn insert<D: DataLayer>(data: &D, resource: &ResourceDef, fields: &[(&str, Value)]) -> Uuid {
    let id = Uuid::new_v4();
    let mut map: FieldMap = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    map.insert("id".into(), Value::Uuid(id));
    data.create(resource, None, id, map).await.unwrap();
    id
}

async fn scenario<D: DataLayer>(data: D) {
    // Names unique to this run, so a shared database's other rows don't count.
    let run = Uuid::new_v4().simple().to_string();
    let name = |n: &str| format!("{run}-{n}");
    let a = insert(&data, &LIBRARY, &[("name", Value::String(name("a")))]).await;
    let b = insert(&data, &LIBRARY, &[("name", Value::String(name("b")))]).await;
    let c = insert(&data, &LIBRARY, &[("name", Value::String(name("c")))]).await;
    for (library, title, kind, pages, archived) in [
        (a, "A1", "long", 300, false),
        (a, "A2", "short", 100, false),
        (a, "A3", "long", 200, false),
        (a, "A4", "long", 1000, true),
        (c, "C1", "short", 50, false),
    ] {
        let fields = [
            ("library_id", Value::Uuid(library)),
            ("title", Value::from(title)),
            ("kind", Value::from(kind)),
            ("pages", Value::Int(pages)),
            ("archived", Value::Bool(archived)),
        ];
        insert(&data, &BOOK, &fields).await;
    }
    let mut readers = Vec::new();
    for _ in 0..2 {
        let reader = insert(&data, &READER, &[]).await;
        insert(&data, &MEMBERSHIP, &[("library_id", Value::Uuid(a)), ("reader_id", Value::Uuid(reader))]).await;
        readers.push(reader);
    }

    let ours = Filter::starts_with("name", run.clone());
    let by_name = vec![Sort { field: "name".into(), descending: false, guard: None }];
    let read = |query: CompiledQuery| {
        let data = &data;
        async move { data.run_query(&LIBRARY, &query).await.unwrap() }
    };
    let values = |rows: &[FieldMap], field: &str| -> Vec<Value> {
        rows.iter().map(|row| row.get(field).cloned().unwrap_or(Value::Null)).collect()
    };

    // Every kind, for libraries with and without related rows, in the query's order.
    let rows = read(CompiledQuery {
        filter: Some(ours.clone()),
        sort: by_name.clone(),
        aggregates: ALL.iter().map(|name| name.to_string()).collect(),
        ..CompiledQuery::default()
    })
    .await;
    assert_eq!(values(&rows, "id"), [Value::Uuid(a), Value::Uuid(b), Value::Uuid(c)]);
    assert_eq!(values(&rows, "book_count"), [Value::Int(3), Value::Int(0), Value::Int(1)]);
    assert_eq!(values(&rows, "long_count"), [Value::Int(2), Value::Int(0), Value::Int(0)]);
    assert_eq!(values(&rows, "pages"), [Value::Int(600), Value::Null, Value::Int(50)]);
    assert_eq!(values(&rows, "has_books"), [Value::Bool(true), Value::Bool(false), Value::Bool(true)]);
    // No actor reads any membership.
    assert_eq!(values(&rows, "reader_count"), [Value::Int(0), Value::Int(0), Value::Int(0)]);
    assert!(matches!(&values(&rows, "a_title")[0], Value::String(t) if ["A1", "A2", "A3"].contains(&t.as_str())));
    assert_eq!(values(&rows, "a_title")[1], Value::Null);

    // A page of them, with only what's selected.
    let rows = read(CompiledQuery {
        filter: Some(ours.clone()),
        sort: by_name,
        select: Some(vec!["name".into()]),
        aggregates: vec!["book_count".into(), "pages".into()],
        limit: Some(2),
        offset: Some(1),
        ..CompiledQuery::default()
    })
    .await;
    assert_eq!(values(&rows, "id"), [Value::Uuid(b), Value::Uuid(c)]);
    assert_eq!(values(&rows, "book_count"), [Value::Int(0), Value::Int(1)]);
    assert_eq!(values(&rows, "pages"), [Value::Null, Value::Int(50)]);
    assert_eq!(values(&rows, "name"), [Value::String(name("b")), Value::String(name("c"))]);

    // Sorted by an aggregate, loaded or not.
    let rows = read(CompiledQuery {
        filter: Some(ours),
        sort: vec![
            Sort { field: "book_count".into(), descending: true, guard: None },
            Sort { field: "name".into(), descending: false, guard: None },
        ],
        aggregates: vec!["reader_count".into()],
        limit: Some(2),
        actor: Some(Actor::new(readers[0])),
        ..CompiledQuery::default()
    })
    .await;
    assert_eq!(values(&rows, "id"), [Value::Uuid(a), Value::Uuid(c)]);
    // The reader reads its own membership, not the other's.
    assert_eq!(values(&rows, "reader_count"), [Value::Int(1), Value::Int(0)]);
}

#[tokio::test]
async fn aggregates_load_laterally_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&LIBRARY, &BOOK, &READER, &MEMBERSHIP]).await.unwrap();
    scenario(pg).await;
}

#[tokio::test]
async fn aggregates_load_grouped_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&LIBRARY, &BOOK, &READER, &MEMBERSHIP]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn aggregates_load_in_memory() {
    scenario(Memory::new()).await;
}
