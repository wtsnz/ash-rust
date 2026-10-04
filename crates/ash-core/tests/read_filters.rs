//! Reads through a relationship apply the destination's primary-read filters, the same
//! way in memory and SQL: in filters, aggregates, and many_to_many joins, on
//! calculations, and without recursing when a read filter leads back to its resource.

use std::collections::HashMap;

use ash_core::{
    ActionDef, AggregateDef, ArgumentDef, AttrType, AttributeDef, CalculationDef, CompiledQuery,
    DataLayer, Expr, FieldMap, Filter, PreparationDef, RelationshipDef, ResourceDef, Value,
};
use ash_memory::Memory;
use ash_sqlite::Sqlite;
use uuid::Uuid;

macro_rules! def {
    ($name:literal, $table:literal, $attrs:expr, $rels:expr, $actions:expr, $calcs:expr, $aggs:expr) => {
        ResourceDef {
            name: $name,
            table: $table,
            attributes: $attrs,
            relationships: $rels,
            actions: $actions,
            policies: &[],
            field_policies: &[],
            calculations: $calcs,
            aggregates: $aggs,
            extensions: &[],
            notifiers: &[],
            identities: &[],
            indexes: &[],
            checks: &[],
            statements: &[],
            embedded: false,
            data_layer: ash_core::DataLayerKind::Memory,
            timestamps: None,
            store_type_id: ash_core::default_store_type_id,
            store_name: "default",
            multitenancy: None,
        }
    };
}

// A category is hidden while it has a child named "bad": a read filter on itself.
fn no_bad_children() -> Filter {
    !Filter::related("children", Filter::eq("name", "bad"))
}
static CATEGORY_DEF: ResourceDef = def!(
    "Category",
    "categories",
    &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("name", AttrType::String),
        AttributeDef::optional("parent_id", AttrType::Uuid),
    ],
    &[RelationshipDef::has_many("children", || &CATEGORY_DEF, "parent_id")],
    &[ActionDef::read("read")
        .primary()
        .preparations(&[PreparationDef::Filter(no_bad_children)])],
    &[],
    &[AggregateDef::count("child_count", "children")]
);

// Archived join rows unlink posts from tags.
fn live_links() -> Filter {
    Filter::is_nil("archived_at")
}
static TAG_DEF: ResourceDef = def!(
    "Tag",
    "tags",
    &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    &[],
    &[ActionDef::read("read").primary()],
    &[],
    &[]
);
static POST_TAG_DEF: ResourceDef = def!(
    "PostTag",
    "post_tags",
    &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("post_id", AttrType::Uuid),
        AttributeDef::required("tag_id", AttrType::Uuid),
        AttributeDef::optional("archived_at", AttrType::String),
    ],
    &[],
    &[ActionDef::read("read")
        .primary()
        .preparations(&[PreparationDef::Filter(live_links)])],
    &[],
    &[]
);
static POST_DEF: ResourceDef = def!(
    "Post",
    "posts",
    &[AttributeDef::uuid_pk("id"), AttributeDef::required("title", AttrType::String)],
    &[RelationshipDef::many_to_many("tags", || &TAG_DEF, || &POST_TAG_DEF, "post_id", "tag_id")],
    &[ActionDef::read("read").primary()],
    &[],
    &[AggregateDef::count("tag_count", "tags")]
);

// Items hide themselves through a calculation, and take a calculation argument.
fn doubled_above_two() -> Filter {
    Filter::gt("double_qty", 2)
}
static ITEM_DEF: ResourceDef = def!(
    "Item",
    "items",
    &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("owner_id", AttrType::Uuid),
        AttributeDef::required("qty", AttrType::Integer),
    ],
    &[],
    &[ActionDef::read("read")
        .primary()
        .preparations(&[PreparationDef::Filter(doubled_above_two)])],
    &[
        CalculationDef::new(
            "double_qty",
            AttrType::Integer,
            Expr::Mul(&Expr::Field("qty"), &Expr::LitInt(2)),
        ),
        CalculationDef::with_arguments(
            "plus",
            AttrType::Integer,
            Expr::Add(&Expr::Field("qty"), &Expr::Arg("extra")),
            &[ArgumentDef::new("extra", AttrType::Integer)],
        ),
    ],
    &[]
);
static OWNER_DEF: ResourceDef = def!(
    "Owner",
    "owners",
    &[AttributeDef::uuid_pk("id")],
    &[RelationshipDef::has_many("items", || &ITEM_DEF, "owner_id")],
    &[ActionDef::read("read").primary()],
    &[],
    &[AggregateDef::count("item_count", "items")]
);

const ALL: [&ResourceDef; 6] = [&CATEGORY_DEF, &TAG_DEF, &POST_TAG_DEF, &POST_DEF, &ITEM_DEF, &OWNER_DEF];

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

async fn insert<D: DataLayer>(data: &D, def: &ResourceDef, n: u128, values: &[(&str, Value)]) {
    let mut fields = FieldMap::new();
    fields.insert("id".into(), Value::Uuid(id(n)));
    for (name, value) in values {
        fields.insert((*name).into(), value.clone());
    }
    data.create(def, None, ash_core::Value::from(id(n)), fields).await.unwrap();
}

async fn seed<D: DataLayer>(data: &D) {
    // Root has children A ("bad") and B ("ok"); B has child C ("bad").
    insert(data, &CATEGORY_DEF, 1, &[("name", "root".into())]).await;
    insert(data, &CATEGORY_DEF, 2, &[("name", "bad".into()), ("parent_id", id(1).into())]).await;
    insert(data, &CATEGORY_DEF, 3, &[("name", "ok".into()), ("parent_id", id(1).into())]).await;
    insert(data, &CATEGORY_DEF, 4, &[("name", "bad".into()), ("parent_id", id(3).into())]).await;

    insert(data, &TAG_DEF, 10, &[("name", "t".into())]).await;
    insert(data, &POST_DEF, 11, &[("title", "unlinked".into())]).await;
    insert(data, &POST_DEF, 12, &[("title", "linked".into())]).await;
    let link = |post: u128| vec![("post_id", Value::from(id(post))), ("tag_id", id(10).into())];
    let mut archived = link(11);
    archived.push(("archived_at", "2024-01-01".into()));
    insert(data, &POST_TAG_DEF, 13, &archived).await;
    insert(data, &POST_TAG_DEF, 14, &link(12)).await;

    insert(data, &OWNER_DEF, 20, &[]).await;
    for (n, qty) in [(21, 1), (22, 2)] {
        insert(data, &ITEM_DEF, n, &[("owner_id", id(20).into()), ("qty", Value::Int(qty))]).await;
    }
}

/// `(id, aggregate)` for each row the query returns, in id order.
async fn run<D: DataLayer>(
    data: &D,
    def: &ResourceDef,
    filter: Option<Filter>,
    aggregate: Option<&str>,
    calculation_args: HashMap<String, FieldMap>,
) -> Vec<(u128, Option<Value>)> {
    let query = CompiledQuery {
        filter,
        aggregates: aggregate.into_iter().map(str::to_string).collect(),
        calculation_args,
        ..CompiledQuery::default()
    };
    let mut rows: Vec<(u128, Option<Value>)> = data
        .run_query(def, &query)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let Some(Value::Uuid(id)) = row.get("id") else {
                panic!("row without id: {row:?}");
            };
            (id.as_u128(), aggregate.and_then(|name| row.get(name).cloned()))
        })
        .collect();
    rows.sort_by_key(|(id, _)| *id);
    rows
}

async fn scenario<D: DataLayer>(data: D) {
    seed(&data).await;
    let none = HashMap::new;

    // The category read filter is applied once inside itself, then left out.
    assert_eq!(
        run(&data, &CATEGORY_DEF, Some(no_bad_children()), None, none()).await,
        [(2, None), (4, None)]
    );
    assert_eq!(
        run(&data, &CATEGORY_DEF, None, Some("child_count"), none()).await,
        [
            (1, Some(Value::Int(1))),
            (2, Some(Value::Int(0))),
            (3, Some(Value::Int(1))),
            (4, Some(Value::Int(0))),
        ]
    );

    // The archived join row neither counts nor matches.
    assert_eq!(
        run(&data, &POST_DEF, None, Some("tag_count"), none()).await,
        [(11, Some(Value::Int(0))), (12, Some(Value::Int(1)))]
    );
    let tagged = Filter::related("tags", Filter::eq("name", "t"));
    assert_eq!(run(&data, &POST_DEF, Some(tagged), None, none()).await, [(12, None)]);

    // The item read filter is on a calculation, which memory must compute to apply it.
    assert_eq!(
        run(&data, &OWNER_DEF, None, Some("item_count"), none()).await,
        [(20, Some(Value::Int(1)))]
    );
    let small = Filter::related("items", Filter::eq("qty", 1));
    assert!(run(&data, &OWNER_DEF, Some(small), None, none()).await.is_empty());

    // A calculation in a filter gets the query's arguments for it.
    let mut extra = FieldMap::new();
    extra.insert("extra".into(), Value::Int(9));
    let args = HashMap::from([("plus".to_string(), extra)]);
    assert_eq!(
        run(&data, &ITEM_DEF, Some(Filter::gt("plus", 10)), None, args).await,
        [(22, None)]
    );
}

#[tokio::test]
async fn read_filters_in_memory() {
    scenario(Memory::new()).await;
}

#[tokio::test]
async fn read_filters_in_sqlite() {
    let sqlite = Sqlite::memory().await.unwrap();
    sqlite.install(&ALL).await.unwrap();
    scenario(sqlite).await;
}
