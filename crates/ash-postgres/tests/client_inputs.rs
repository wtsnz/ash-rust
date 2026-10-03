//! A client's filters and sorts as Ash runs them, in memory, SQLite and Postgres alike:
//! a field a field policy hides reads as null where it's hidden, so neither finds nor
//! orders records by values the actor can't see; and keyset pages walk nullable sorts,
//! nulls last ascending and first descending unless a sort places them otherwise.

use ash_core::{
    Actor, CompiledQuery, Context, DataLayer, FieldMap, Filter, Resource, Sort, Value, build_keyset_filter,
    guard_input_filter, guard_input_sort, keyset_sort, redact_fields,
};
use ash_memory::Memory;
use ash_postgres::Postgres;
use ash_sqlite::Sqlite;
use uuid::Uuid;

mod notes {
    use ash_core::resource;
    use uuid::Uuid;

    resource! {
        Note {
            table "input_notes";

            actor {
                role: String;
            }

            attributes {
                id: Uuid [pk];
                owner_id: Uuid;
                rank: Option<i64>;
                secret: Option<String>;
            }

            field_policies {
                field secret {
                    authorize_if relates_to_actor(owner_id);
                    authorize_if actor_attribute_equals(role, "admin");
                }
            }

            actions {
                read read { primary; }
                create create { primary; accept [id, owner_id, rank, secret]; }
            }
        }
    }
}

use notes::Note;

fn member(id: Uuid) -> Actor {
    Actor::new(id).with_attr("role", Value::from("member"))
}

fn sort(field: &str, descending: bool) -> Sort {
    Sort { field: field.into(), descending, ..Default::default() }
}

/// The notes `query` reads, as `actor` may see them.
async fn read<D: DataLayer>(data: &D, actor: &Actor, query: CompiledQuery) -> Vec<FieldMap> {
    let mut rows = data.run_query(&Note::DEF, &query).await.unwrap();
    for row in &mut rows {
        redact_fields(&Note::DEF, Some(actor), row).unwrap();
    }
    rows
}

/// The ids `filter`, from a client, finds as `actor` sees it among the notes `scope`
/// holds (this run's: Postgres keeps earlier runs' rows), by id.
async fn found<D: DataLayer>(data: &D, actor: &Actor, scope: &Filter, filter: Filter) -> Vec<Uuid> {
    let filter = guard_input_filter(&Note::DEF, Some(actor), filter).unwrap();
    let filter = Filter::and([scope.clone(), filter]);
    let query = CompiledQuery { filter: Some(filter), sort: vec![sort("id", false)], ..CompiledQuery::default() };
    read(data, actor, query).await.iter().map(|row| row["id"].as_uuid().unwrap()).collect()
}

/// Every note `scope` holds, page by page of two, after (or before) the last page's last
/// (or first) record, in `sorts` from a client, as `actor` reads them.
async fn walk<D: DataLayer>(data: &D, actor: &Actor, scope: &Filter, sorts: Vec<Sort>, forward: bool) -> Vec<Uuid> {
    let sorts = keyset_sort(&Note::DEF, guard_input_sort(&Note::DEF, Some(actor), sorts).unwrap());
    let read_sorts: Vec<Sort> =
        sorts.iter().map(|s| if forward { s.clone() } else { s.reversed() }).collect();
    let mut seen = Vec::new();
    let mut last: Option<FieldMap> = None;
    loop {
        let filter = last.as_ref().and_then(|row| {
            let values: Vec<Value> = sorts.iter().map(|s| row.get(&s.field).cloned().unwrap_or(Value::Null)).collect();
            build_keyset_filter(&Note::DEF, &sorts, &values, forward)
        });
        let filter = Some(Filter::and([scope.clone()].into_iter().chain(filter)));
        let query = CompiledQuery { filter, sort: read_sorts.clone(), limit: Some(2), ..CompiledQuery::default() };
        let page = read(data, actor, query).await;
        if page.is_empty() {
            break;
        }
        assert!(seen.len() < 10, "the pages don't end: {seen:?}");
        seen.extend(page.iter().map(|row| row["id"].as_uuid().unwrap()));
        last = page.last().cloned();
    }
    if !forward {
        seen.reverse();
    }
    seen
}

async fn scenario<D: DataLayer + Clone + 'static>(data: D) {
    let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    // Ids in order, so ties break predictably.
    let mut ids: Vec<Uuid> = (0..5).map(|_| Uuid::new_v4()).collect();
    ids.sort();
    let [n1, n2, n3, n4, n5] = ids[..] else { unreachable!() };
    let ctx = Context::new(data.clone());
    for (id, owner, rank, secret) in [
        (n1, a, Some(1), Some("alpha")),
        (n2, a, None, Some("beta")),
        (n3, b, Some(3), Some("alpha")),
        (n4, b, Some(2), None),
        (n5, c, None, Some("gamma")),
    ] {
        Note::create(&ctx).id(id).owner_id(owner).rank(rank).secret(secret.map(str::to_string)).await.unwrap();
    }
    let (owner, admin) = (member(a), Actor::new(Uuid::new_v4()).with_attr("role", Value::from("admin")));
    let scope = &Filter::in_list("owner_id", vec![Value::Uuid(a), Value::Uuid(b), Value::Uuid(c)]);

    // Only the secrets the actor may read can be found.
    assert_eq!(found(&data, &owner, scope, Filter::eq("secret", "alpha")).await, [n1]);
    assert_eq!(found(&data, &admin, scope, Filter::eq("secret", "alpha")).await, [n1, n3]);
    // Negated too: a hidden secret is neither "alpha" nor not.
    assert_eq!(found(&data, &owner, scope, !Filter::eq("secret", "alpha")).await, [n2]);
    assert_eq!(found(&data, &owner, scope, Filter::or([Filter::eq("secret", "beta"), Filter::eq("rank", 3)])).await, [n2, n3]);
    // A hidden secret reads as nil.
    assert_eq!(found(&data, &owner, scope, Filter::is_nil("secret")).await, [n3, n4, n5]);
    assert_eq!(found(&data, &owner, scope, !Filter::is_nil("secret")).await, [n1, n2]);
    assert_eq!(found(&data, &admin, scope, Filter::is_nil("secret")).await, [n4]);

    // Nulls last ascending, first descending, page by page either way.
    for forward in [true, false] {
        assert_eq!(walk(&data, &owner, scope, vec![sort("rank", false)], forward).await, [n1, n4, n3, n2, n5], "forward {forward}");
        assert_eq!(walk(&data, &owner, scope, vec![sort("rank", true)], forward).await, [n2, n5, n3, n4, n1], "forward {forward}");
        // Or where the sort puts them: Ash's `++rank` and `--rank`.
        let nulls = |descending, first| vec![Sort { nulls_first: Some(first), ..sort("rank", descending) }];
        assert_eq!(walk(&data, &owner, scope, nulls(false, true), forward).await, [n2, n5, n1, n4, n3], "forward {forward}");
        assert_eq!(walk(&data, &owner, scope, nulls(true, false), forward).await, [n3, n4, n1, n2, n5], "forward {forward}");
        // A hidden secret sorts as null: the owner's own first, then the rest by id.
        assert_eq!(walk(&data, &owner, scope, vec![sort("secret", false)], forward).await, [n1, n2, n3, n4, n5], "forward {forward}");
        assert_eq!(walk(&data, &owner, scope, vec![sort("secret", true)], forward).await, [n3, n4, n5, n2, n1], "forward {forward}");
        assert_eq!(walk(&data, &admin, scope, vec![sort("secret", false)], forward).await, [n1, n3, n2, n5, n4], "forward {forward}");
    }
}

#[tokio::test]
async fn client_inputs_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn client_inputs_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&[&Note::DEF]).await.unwrap();
    scenario(sqlite).await;
}

#[tokio::test]
async fn client_inputs_in_postgres() {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "postgres://localhost/ash_test".into());
    let Ok(pg) = Postgres::connect(&url).await else {
        assert!(std::env::var("CI").is_err(), "CI runs Postgres");
        eprintln!("PostgreSQL not reachable; skipping test");
        return;
    };
    pg.install(&[&Note::DEF]).await.unwrap();
    scenario(pg).await;
}
