//! A relationship's limit and offset page each source's rows, however the data layer
//! reads them: Postgres once per source in one statement, with a lateral join, as
//! AshPostgres loads a relationship; SQLite and memory all at once, paging each source's
//! rows after, as Ash does without lateral joins. One scenario, the same answers.

use ash_core::{
    ActionDef, AttrType, AttributeDef, Context, DataLayer, FieldMap, Filter, PreparationDef, RelatedQuery,
    RelationshipDef, ResourceDef, Sort, Value, load_related_query,
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

static SHELF: ResourceDef = ResourceDef {
    name: "RelShelf",
    table: "rel_shelves",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    relationships: &[
        RelationshipDef::has_many("books", || &BOOK, "shelf_id"),
        RelationshipDef::many_to_many("readers", || &READER, || &LOAN, "shelf_id", "reader_id"),
    ],
    actions: &[ActionDef::read("read").primary()],
    ..BASE
};

fn on_shelf() -> Filter {
    Filter::eq("archived", false)
}

/// Archived books are out of the primary read's reach, so no load returns them.
static BOOK: ResourceDef = ResourceDef {
    name: "RelBook",
    table: "rel_books",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("shelf_id", AttrType::Uuid),
        AttributeDef::required("title", AttrType::String),
        AttributeDef::required("pages", AttrType::Integer),
        AttributeDef::required("archived", AttrType::Boolean),
    ],
    actions: &[ActionDef::read("read").primary().preparations(&[PreparationDef::Filter(on_shelf)])],
    ..BASE
};

static READER: ResourceDef = ResourceDef {
    name: "RelReader",
    table: "rel_readers",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    actions: &[ActionDef::read("read").primary()],
    ..BASE
};

static LOAN: ResourceDef = ResourceDef {
    name: "RelLoan",
    table: "rel_loans",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("shelf_id", AttrType::Uuid),
        AttributeDef::required("reader_id", AttrType::Uuid),
    ],
    actions: &[ActionDef::read("read").primary()],
    ..BASE
};

async fn insert<D: DataLayer>(data: &D, resource: &ResourceDef, fields: &[(&str, Value)]) -> FieldMap {
    let id = Uuid::new_v4();
    let mut map: FieldMap = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    map.insert("id".into(), Value::Uuid(id));
    data.create(resource, None, id, map).await.unwrap()
}

fn texts(rows: &[FieldMap], field: &str) -> Vec<String> {
    rows.iter().map(|row| row.get(field).and_then(Value::as_str).unwrap_or("?").to_string()).collect()
}

async fn scenario<D: DataLayer + Clone>(data: D) {
    let a = insert(&data, &SHELF, &[("name", Value::from("a"))]).await;
    let b = insert(&data, &SHELF, &[("name", Value::from("b"))]).await;
    let empty = insert(&data, &SHELF, &[("name", Value::from("empty"))]).await;
    let id = |shelf: &FieldMap| shelf.get("id").cloned().unwrap();
    for (shelf, title, pages, archived) in [
        (&a, "A1", 100, false),
        (&a, "A2", 300, false),
        (&a, "A3", 200, false),
        (&a, "A0", 900, true),
        (&b, "B1", 50, false),
        (&b, "B2", 70, false),
    ] {
        let fields = [
            ("shelf_id", id(shelf)),
            ("title", Value::from(title)),
            ("pages", Value::Int(pages)),
            ("archived", Value::Bool(archived)),
        ];
        insert(&data, &BOOK, &fields).await;
    }
    for (shelf, name) in [(&a, "Ann"), (&a, "Bob"), (&a, "Cat"), (&b, "Dee")] {
        let reader = insert(&data, &READER, &[("name", Value::from(name))]).await;
        insert(&data, &LOAN, &[("shelf_id", id(shelf)), ("reader_id", id(&reader))]).await;
    }
    let ctx = Context::new(data);
    let shelves = [a.clone(), b.clone(), empty, a.clone()];
    let load = |relationship: &'static str, query: RelatedQuery| {
        let ctx = ctx.clone();
        let shelves = shelves.clone();
        async move { load_related_query(&ctx, &SHELF, relationship, &shelves, &query).await.unwrap() }
    };
    let by = |field: &str, descending: bool| vec![Sort { field: field.into(), descending }];

    // The longest two of each shelf's books; the same shelf twice gets them twice.
    let books = load("books", RelatedQuery { sort: by("pages", true), limit: Some(2), ..RelatedQuery::default() }).await;
    let titles: Vec<Vec<String>> = books.iter().map(|rows| texts(rows, "title")).collect();
    assert_eq!(titles, [vec!["A2", "A3"], vec!["B2", "B1"], vec![], vec!["A2", "A3"]]);

    // Past the first, filtered, and only what's selected (with the key it links on).
    let books = load(
        "books",
        RelatedQuery {
            sort: by("title", false),
            offset: Some(1),
            filter: Some(Filter::gt("pages", Value::Int(60))),
            select: Some(vec!["title".into()]),
            ..RelatedQuery::default()
        },
    )
    .await;
    let titles: Vec<Vec<String>> = books.iter().map(|rows| texts(rows, "title")).collect();
    assert_eq!(titles, [vec!["A2", "A3"], vec![], vec![], vec!["A2", "A3"]]);
    for row in books.iter().flatten() {
        let mut keys: Vec<&str> = row.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["id", "shelf_id", "title"]);
    }

    // Through a join resource, paged for each shelf too.
    let readers = load("readers", RelatedQuery { sort: by("name", true), limit: Some(2), ..RelatedQuery::default() }).await;
    let names: Vec<Vec<String>> = readers.iter().map(|rows| texts(rows, "name")).collect();
    assert_eq!(names, [vec!["Cat", "Bob"], vec!["Dee"], vec![], vec!["Cat", "Bob"]]);
    let readers = load("readers", RelatedQuery { sort: by("name", false), offset: Some(1), ..RelatedQuery::default() }).await;
    let names: Vec<Vec<String>> = readers.iter().map(|rows| texts(rows, "name")).collect();
    assert_eq!(names, [vec!["Bob", "Cat"], vec![], vec![], vec!["Bob", "Cat"]]);
}

#[tokio::test]
async fn relationship_pages_load_laterally_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    assert!(pg.can_join_laterally(&BOOK));
    pg.install(&[&SHELF, &BOOK, &READER, &LOAN]).await.unwrap();
    scenario(pg).await;
}

#[tokio::test]
async fn relationship_pages_load_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    assert!(!sqlite.can_join_laterally(&BOOK));
    sqlite.install(&[&SHELF, &BOOK, &READER, &LOAN]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn relationship_pages_load_in_memory() {
    scenario(Memory::new()).await;
}
