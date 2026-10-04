//! Typed maps and unions are stored as JSON and read back as their types, in every data
//! layer, as Ash stores a typed map and a union (`{type, value}`).

use ash_core::{AshTypedMap, AshUnion, Context, DataLayer, Filter, Resource, resource};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, AshTypedMap)]
pub struct Point {
    pub x: i64,
    pub y: i64,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, PartialEq, AshUnion)]
pub enum Shape {
    Dot(Point),
    #[ash(rename = "label")]
    Text(String),
    Points(Vec<Point>),
}

resource! {
    Sketch {
        table "composite_value_sketches";

        attributes {
            id: Uuid [pk];
            run: Uuid;
            origin: Point;
            shape: Option<Shape>;
        }

        actions {
            read read { primary; }
            create create { primary; accept [run, origin, shape]; }
        }
    }
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    let ctx = Context::new(data);
    let run = Uuid::new_v4();
    let origin = Point { x: 1, y: -2, tag: Some("o".into()) };
    for shape in [
        Some(Shape::Dot(Point { x: 3, y: 4, tag: None })),
        Some(Shape::Text("hi".into())),
        Some(Shape::Points(vec![Point { x: 0, y: 0, tag: None }, Point { x: 5, y: 6, tag: Some("p".into()) }])),
        None,
    ] {
        let created = Sketch::create(&ctx).run(run).origin(origin.clone()).shape(shape.clone()).await.unwrap();
        assert_eq!(created.shape, shape);
        let read = Sketch::query(&ctx).filter(Filter::eq("id", created.id)).one().await.unwrap();
        assert_eq!((read.origin, read.shape), (origin.clone(), shape));
    }
    assert_eq!(Sketch::query(&ctx).filter(Filter::eq("run", run)).count().await.unwrap(), 4);
}

#[test]
fn the_types_declare_their_fields_and_members() {
    use ash_core::{AshType, AttrType};
    let AttrType::TypedMap(fields) = Point::ATTR_TYPE else { panic!() };
    assert_eq!(fields.iter().map(|f| (f.name, f.allow_nil)).collect::<Vec<_>>(), [("x", false), ("y", false), ("tag", true)]);
    let AttrType::Union(members) = Shape::ATTR_TYPE else { panic!() };
    assert_eq!(members.iter().map(|m| m.name).collect::<Vec<_>>(), ["dot", "label", "points"]);
}

#[tokio::test]
async fn composite_values_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn composite_values_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Sketch::DEF]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn composite_values_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&Sketch::DEF]).await.unwrap();
    scenario(pg).await;
}
