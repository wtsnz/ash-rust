//! Relationships load ahead of their fields, as AshGraphql loads them: one read per
//! selected relationship, whatever the number of records, nested selections included,
//! and only through what the requester may see.

use std::sync::{Arc, Mutex};

use ash_core::{
    ActionDef, Actor, AggregateDef, AttrType, CalculationDef, Expr, AttributeDef, Check, CompiledQuery, Context, DataLayer, FieldMap,
    FieldPolicyDef, OnDelete, OnUpdate, PolicyEffect, RelKind, RelationshipDef, ResourceDef, Result,
    Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value as Json, json};
use uuid::Uuid;

/// A read's resource, and the attributes it selected.
type Select = (&'static str, Option<Vec<String>>);

/// Memory that notes which resource each read is of, and what it selects.
#[derive(Clone, Default)]
struct Counting {
    inner: Memory,
    reads: Arc<Mutex<Vec<&'static str>>>,
    selects: Arc<Mutex<Vec<Select>>>,
}

impl Counting {
    /// Each read's resource and the attributes it selected, in order.
    fn take_selects(&self) -> Vec<Select> {
        let mut selects = std::mem::take(&mut *self.selects.lock().unwrap());
        for (_, select) in &mut selects {
            if let Some(select) = select {
                select.sort();
            }
        }
        selects.sort();
        selects
    }

    fn take(&self) -> Vec<&'static str> {
        let mut reads = std::mem::take(&mut *self.reads.lock().unwrap());
        reads.sort_unstable();
        reads
    }
}

impl DataLayer for Counting {
    async fn create(&self, resource: &ResourceDef, tenant: Option<&str>, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        self.inner.create(resource, tenant, id, fields).await
    }

    async fn update(&self, resource: &ResourceDef, tenant: Option<&str>, id: Uuid, fields: FieldMap) -> Result<FieldMap> {
        self.inner.update(resource, tenant, id, fields).await
    }

    async fn destroy(&self, resource: &ResourceDef, tenant: Option<&str>, id: Uuid) -> Result<()> {
        self.inner.destroy(resource, tenant, id).await
    }

    fn can_update_atomically(&self, resource: &ResourceDef) -> bool {
        self.inner.can_update_atomically(resource)
    }

    async fn update_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        update: &ash_core::AtomicUpdate,
    ) -> Result<Vec<FieldMap>> {
        self.reads.lock().unwrap().push("atomic update");
        self.inner.update_atomic(resource, query, update).await
    }

    fn can_destroy_atomically(&self, resource: &ResourceDef) -> bool {
        self.inner.can_destroy_atomically(resource)
    }

    async fn destroy_atomic(
        &self,
        resource: &ResourceDef,
        query: &CompiledQuery,
        conditions: &[ash_core::AtomicCondition],
    ) -> Result<Vec<FieldMap>> {
        self.reads.lock().unwrap().push("atomic destroy");
        self.inner.destroy_atomic(resource, query, conditions).await
    }

    async fn run_query(&self, resource: &ResourceDef, query: &CompiledQuery) -> Result<Vec<FieldMap>> {
        let name = [&AUTHOR_DEF, &POST_DEF, &COMMENT_DEF]
            .into_iter()
            .find(|def| def.name == resource.name)
            .map(|def| def.name)
            .unwrap_or("other");
        self.reads.lock().unwrap().push(name);
        self.selects.lock().unwrap().push((name, query.select.clone()));
        self.inner.run_query(resource, query).await
    }
}

const fn rel(
    name: &'static str,
    kind: RelKind,
    destination: fn() -> &'static ResourceDef,
    source_attribute: &'static str,
    destination_attribute: &'static str,
) -> RelationshipDef {
    RelationshipDef {
        name,
        kind,
        destination,
        source_attribute,
        destination_attribute,
        source_attributes: &[],
        destination_attributes: &[],
        through: None,
        source_attribute_on_join_resource: None,
        destination_attribute_on_join_resource: None,
        on_delete: OnDelete::Nothing,
        on_update: OnUpdate::Nothing,
    }
}

static AUTHOR_DEF: ResourceDef = ResourceDef {
    name: "Author",
    table: "authors",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    relationships: &[
        rel("posts", RelKind::HasMany, || &POST_DEF, "id", "author_id"),
        // One of the author's posts, though they've written several.
        rel("one_post", RelKind::HasOne, || &POST_DEF, "id", "author_id"),
    ],
    actions: &[ActionDef::read("read").primary().pagination(ash_core::Pagination::keyset().countable(ash_core::Countable::Yes).required(false))],
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
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

/// Who wrote a post, and how many comments it has, are for signed-in readers only.
/// A post's label is no one's to read, and a labelled post's comment count isn't either.
static POST_FIELD_POLICIES: &[FieldPolicyDef] = &[
    FieldPolicyDef::new("author_id", &[PolicyEffect::AuthorizeIf(Check::ActorPresent)]),
    FieldPolicyDef::new("label", &[PolicyEffect::ForbidIf(Check::Always)]),
    FieldPolicyDef::new(
        "comment_count",
        &[PolicyEffect::AuthorizeIf(Check::And(&[Check::ActorPresent, Check::IsNil { field: "label" }]))],
    ),
];

static IS_FIRST: Expr = Expr::Eq(&Expr::Field("title"), &Expr::LitString("A0P0"));
static LABEL: Expr = Expr::Field("label");
static OTHER: Expr = Expr::LitString("other");

fn shout(fields: &FieldMap) -> Result<Value> {
    Ok(fields.get("title").and_then(Value::as_str).map(|t| Value::String(t.to_uppercase())).unwrap_or(Value::Null))
}

static POST_DEF: ResourceDef = ResourceDef {
    name: "Post",
    table: "posts",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("title", AttrType::String),
        AttributeDef::required("author_id", AttrType::Uuid),
        AttributeDef::optional("label", AttrType::String),
    ],
    relationships: &[
        rel("author", RelKind::BelongsTo, || &AUTHOR_DEF, "author_id", "id"),
        rel("comments", RelKind::HasMany, || &COMMENT_DEF, "id", "post_id"),
    ],
    actions: &[
        ActionDef::read("read").primary().pagination(ash_core::Pagination::keyset().countable(ash_core::Countable::Yes).required(false)),
        ActionDef::create("create").accept(&["title", "author_id"]),
        ActionDef::update("retitle").accept(&["title"]),
        ActionDef::destroy("destroy"),
    ],
    aggregates: &[AggregateDef::count("comment_count", "comments")],
    calculations: &[
        // The first post's label, which it hasn't got; any other's "other".
        CalculationDef::new(
            "first_label",
            AttrType::String,
            Expr::IfElse { cond: &IS_FIRST, then_expr: &LABEL, else_expr: &OTHER },
        ),
        // Only Rust can compute this.
        CalculationDef::new("shout", AttrType::String, Expr::Custom(shout)),
    ],
    field_policies: POST_FIELD_POLICIES,
    ..AUTHOR_DEF
};

static COMMENT_DEF: ResourceDef = ResourceDef {
    name: "Comment",
    table: "comments",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("body", AttrType::String),
        AttributeDef::required("post_id", AttrType::Uuid),
    ],
    relationships: &[rel("post", RelKind::BelongsTo, || &POST_DEF, "post_id", "id")],
    ..AUTHOR_DEF
};

async fn insert(data: &Counting, def: &ResourceDef, fields: &[(&str, Value)]) -> Uuid {
    let id = Uuid::new_v4();
    let mut map: FieldMap = fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
    map.insert("id".into(), Value::Uuid(id));
    data.create(def, None, id, map).await.unwrap();
    id
}

/// Three authors, two posts each, two comments on each post.
async fn seeded() -> (Counting, Uuid) {
    let data = Counting::default();
    let mut first_post = Uuid::nil();
    for a in 0..3 {
        let author = insert(&data, &AUTHOR_DEF, &[("name", Value::String(format!("A{a}")))]).await;
        for p in 0..2 {
            let post = insert(
                &data,
                &POST_DEF,
                &[("title", Value::String(format!("A{a}P{p}"))), ("author_id", Value::Uuid(author))],
            )
            .await;
            if first_post.is_nil() {
                first_post = post;
            }
            for c in 0..2 {
                insert(&data, &COMMENT_DEF, &[("body", Value::String(format!("A{a}P{p}C{c}"))), ("post_id", Value::Uuid(post))])
                    .await;
            }
        }
    }
    data.take();
    (data, first_post)
}

async fn run(data: &Counting, query: &str, actor: Option<Actor>) -> Json {
    let schema = AshGraphQL::from_resources(&[&AUTHOR_DEF, &POST_DEF, &COMMENT_DEF])
        .finish::<Counting>()
        .unwrap();
    let mut ctx = Context::new(data.clone());
    if let Some(actor) = actor {
        ctx = ctx.with_actor(actor);
    }
    let res = schema.execute(Request::new(query).data(ctx)).await;
    // Fields a policy hides are null, and reported so.
    let forbidden = |e: &async_graphql::ServerError| e.message == "forbidden field";
    assert!(res.errors.iter().all(forbidden), "{:?}", res.errors);
    res.data.into_json().unwrap()
}

#[tokio::test]
async fn each_selected_relationship_is_one_read_however_many_records() {
    let (data, _) = seeded().await;
    let actor = Some(Actor::new(Uuid::new_v4()));
    let page = run(
        &data,
        "{ listAuthors { results { name posts { title comments { body } author { name } } } } }",
        actor,
    )
    .await;
    // The authors, their posts, the posts' comments and the posts' authors: a read each.
    assert_eq!(data.take(), ["Author", "Author", "Comment", "Post"]);

    let authors = page["listAuthors"]["results"].as_array().unwrap();
    assert_eq!(authors.len(), 3);
    for author in authors {
        let name = author["name"].as_str().unwrap();
        let posts = author["posts"].as_array().unwrap();
        assert_eq!(posts.len(), 2);
        for post in posts {
            assert_eq!(post["author"]["name"], name);
            let title = post["title"].as_str().unwrap();
            let comments = post["comments"].as_array().unwrap();
            assert_eq!(comments.len(), 2);
            assert!(comments.iter().all(|c| c["body"].as_str().unwrap().starts_with(title)));
        }
    }
}

#[tokio::test]
async fn aliases_and_fragments_are_loaded_as_selected() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        "{ listPosts(first: 2) { results { ...Post who: author { name } again: author { id } } } }
         fragment Post on Post { title onPost: comments { body } }",
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    assert_eq!(data.take(), ["Author", "Author", "Comment", "Post"]);
    for post in page["listPosts"]["results"].as_array().unwrap() {
        assert!(post["who"]["name"].is_string(), "{post}");
        assert!(post["again"]["id"].is_string(), "{post}");
        assert_eq!(post["onPost"].as_array().unwrap().len(), 2);
    }
}

/// As in Ash, a field policy hides an attribute's value, not the relationships through it:
/// an anonymous reader can't see who wrote a post by its key, but can follow `author`.
#[tokio::test]
async fn a_relationship_loads_through_a_key_a_field_policy_hides() {
    let (data, _) = seeded().await;
    let page = run(&data, "{ listPosts { results { title authorId author { name } } } }", None).await;
    for post in page["listPosts"]["results"].as_array().unwrap() {
        assert!(post["authorId"].is_null(), "{post}");
        assert!(post["author"]["name"].is_string(), "{post}");
    }
}

/// Two pages of the same records, one shaping a relationship with arguments: each gets
/// its own rows.
#[tokio::test]
async fn a_relationship_with_arguments_never_takes_rows_preloaded_without() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        "{ listAuthors { a: results { posts(limit: 1) { title } } b: results { posts { title } } } }",
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    for (a, b) in page["listAuthors"]["a"].as_array().unwrap().iter().zip(page["listAuthors"]["b"].as_array().unwrap()) {
        assert_eq!(a["posts"].as_array().unwrap().len(), 1, "{a}");
        assert_eq!(b["posts"].as_array().unwrap().len(), 2, "{b}");
    }
}

/// The same response key for two relationships, under two pages of the same records.
#[tokio::test]
async fn one_response_key_for_two_relationships_keeps_both() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        "{ listPosts { a: results { x: author { name } } b: results { x: comments { body } } } }",
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    for (a, b) in page["listPosts"]["a"].as_array().unwrap().iter().zip(page["listPosts"]["b"].as_array().unwrap()) {
        assert!(a["x"]["name"].is_string(), "{a}");
        assert_eq!(b["x"].as_array().unwrap().len(), 2, "{b}");
    }
}

/// A relationship selected twice under one response key loads once, with both selections.
#[tokio::test]
async fn repeated_selections_merge() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        "{ listPosts { results { author { name } author { id posts { title } } } } }",
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    assert_eq!(data.take(), ["Author", "Post", "Post"]);
    for post in page["listPosts"]["results"].as_array().unwrap() {
        assert!(post["author"]["name"].is_string() && post["author"]["id"].is_string(), "{post}");
        assert_eq!(post["author"]["posts"].as_array().unwrap().len(), 2, "{post}");
    }
}

/// A has-one whose destination has several rows for a record gives each record its own.
#[tokio::test]
async fn a_has_one_with_several_rows_gives_each_record_its_own() {
    let (data, _) = seeded().await;
    let page = run(&data, "{ listAuthors { results { name onePost { title } } } }", Some(Actor::new(Uuid::new_v4()))).await;
    for author in page["listAuthors"]["results"].as_array().unwrap() {
        let name = author["name"].as_str().unwrap();
        assert!(author["onePost"]["title"].as_str().unwrap().starts_with(name), "{author}");
    }
}

#[tokio::test]
async fn gets_and_mutation_results_load_their_relationships() {
    let (data, post) = seeded().await;
    let actor = Some(Actor::new(Uuid::new_v4()));
    let got = run(&data, &format!(r#"{{ getPost(id: "{post}") {{ title author {{ name }} comments {{ body }} }} }}"#), actor.clone()).await;
    assert!(got["getPost"]["author"]["name"].is_string());
    assert_eq!(got["getPost"]["comments"].as_array().unwrap().len(), 2);

    let author = got["getPost"]["author"]["name"].clone();
    let author_id = run(&data, &format!(r#"{{ getPost(id: "{post}") {{ authorId }} }}"#), actor.clone()).await["getPost"]["authorId"].clone();
    let created = run(
        &data,
        &format!(r#"mutation {{ createPost(input: {{ title: "New", authorId: {author_id} }}) {{ result {{ title author {{ name }} }} errors {{ message }} }} }}"#),
        actor,
    )
    .await;
    assert_eq!(created["createPost"]["result"]["author"]["name"], author);
}

#[tokio::test]
async fn a_relationship_with_arguments_is_still_shaped_by_them() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        r#"{ listAuthors { results { posts(sort: [{ field: TITLE, order: DESC }], limit: 1) { title } } } }"#,
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    for author in page["listAuthors"]["results"].as_array().unwrap() {
        let posts = author["posts"].as_array().unwrap();
        assert_eq!(posts.len(), 1);
        assert!(posts[0]["title"].as_str().unwrap().ends_with("P1"), "{author}");
    }
    assert_eq!(json!(data.take().len() > 1), json!(true));
}

/// An update by id is one atomic statement, as AshGraphql's: no read of the record first.
#[tokio::test]
async fn an_update_is_one_atomic_statement() {
    let (data, post) = seeded().await;
    let page = run(
        &data,
        &format!(r#"mutation {{ retitlePost(id: "{post}", input: {{ title: "Renamed" }}) {{ result {{ title }} errors {{ message }} }} }}"#),
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    assert_eq!(page["retitlePost"]["result"]["title"], "Renamed");
    assert_eq!(data.take(), ["atomic update"]);
}

#[tokio::test]
async fn a_destroy_is_one_atomic_statement() {
    let (data, post) = seeded().await;
    let mutation = format!(r#"mutation {{ destroyPost(id: "{post}") {{ result {{ title }} errors {{ message }} }} }}"#);
    let page = run(&data, &mutation, Some(Actor::new(Uuid::new_v4()))).await;
    assert_eq!(page["destroyPost"]["result"]["title"], "A0P0");
    assert_eq!(data.take(), ["atomic destroy"]);

    // It's gone: the same statement deletes nothing.
    let page = run(&data, &mutation, Some(Actor::new(Uuid::new_v4()))).await;
    assert_eq!(page["destroyPost"]["result"], Json::Null);
    assert_eq!(data.take(), ["atomic destroy"]);
}

/// As AshGraphql loads a relationship with the related query its arguments build: one
/// read for every author, each author's rows then limited.
#[tokio::test]
async fn a_relationship_selected_with_arguments_is_one_read_too() {
    let (data, _) = seeded().await;
    let page = run(
        &data,
        r#"{ listAuthors(sort: [{ field: NAME }]) { results { name
             latest: posts(sort: [{ field: TITLE, order: DESC }], limit: 1) { title comments(sort: [{ field: BODY }], offset: 1) { body } }
             earliest: posts(sort: [{ field: TITLE }], limit: 1) { title }
             named: posts(filter: { title: { eq: "A1P0" } }) { title } } } }"#,
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    // The authors; their posts under each alias; the latest posts' comments.
    assert_eq!(data.take(), ["Author", "Comment", "Post", "Post", "Post"]);

    for author in page["listAuthors"]["results"].as_array().unwrap() {
        let name = author["name"].as_str().unwrap();
        assert_eq!(author["latest"], json!([{ "title": format!("{name}P1"), "comments": [{ "body": format!("{name}P1C1") }] }]));
        assert_eq!(author["earliest"], json!([{ "title": format!("{name}P0") }]));
        let named = if name == "A1" { json!([{ "title": "A1P0" }]) } else { json!([]) };
        assert_eq!(author["named"], named, "{author}");
    }
}

/// Arguments given through variables shape the load as written ones do.
#[tokio::test]
async fn relationship_arguments_may_be_variables() {
    let (data, _) = seeded().await;
    let schema = AshGraphQL::from_resources(&[&AUTHOR_DEF, &POST_DEF, &COMMENT_DEF])
        .finish::<Counting>()
        .unwrap();
    let request = Request::new(
        "query($limit: Int, $sort: [PostSortInput]) { listAuthors { results { posts(limit: $limit, sort: $sort) { title } } } }",
    )
    .variables(async_graphql::Variables::from_json(json!({ "limit": 1, "sort": [{ "field": "TITLE", "order": "DESC" }] })))
    .data(Context::new(data.clone()).with_actor(Actor::new(Uuid::new_v4())));
    let res = schema.execute(request).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(data.take(), ["Author", "Post"]);
    let page = res.data.into_json().unwrap();
    for author in page["listAuthors"]["results"].as_array().unwrap() {
        let posts = author["posts"].as_array().unwrap();
        assert_eq!(posts.len(), 1, "{author}");
        assert!(posts[0]["title"].as_str().unwrap().ends_with("P1"), "{author}");
    }
}

/// As AshGraphql selects only the fields asked for: the attributes selected, the keys
/// of the relationships selected, and the primary key.
#[tokio::test]
async fn a_read_selects_only_what_is_asked_for() {
    let (data, _) = seeded().await;
    run(&data, "{ listPosts(first: 2) { results { title author { name } } } }", Some(Actor::new(Uuid::new_v4()))).await;
    let strings = |names: &[&str]| Some(names.iter().map(|name| name.to_string()).collect::<Vec<_>>());
    assert_eq!(
        data.take_selects(),
        // And what Post's field policies check: its label.
        [("Author", strings(&["id", "name"])), ("Post", strings(&["author_id", "id", "label", "title"]))]
    );
}

/// Aggregates selected load with the records, as AshGraphql loads them, and a read can
/// sort and filter by them; a mutation's result loads those it selects too.
#[tokio::test]
async fn selected_aggregates_load_and_sort_and_filter() {
    let (data, post) = seeded().await;
    insert(&data, &COMMENT_DEF, &[("body", Value::from("A0P0C2")), ("post_id", Value::Uuid(post))]).await;
    let actor = || Some(Actor::new(Uuid::new_v4()));

    let page = run(
        &data,
        "{ listPosts(sort: [{ field: COMMENT_COUNT, order: DESC }, { field: TITLE }], first: 2) { results { title commentCount } } }",
        actor(),
    )
    .await;
    assert_eq!(
        page["listPosts"]["results"],
        json!([{ "title": "A0P0", "commentCount": 3 }, { "title": "A0P1", "commentCount": 2 }])
    );

    let page = run(&data, "{ listPosts(filter: { commentCount: { greaterThan: 2 } }) { results { title } } }", actor()).await;
    assert_eq!(page["listPosts"]["results"], json!([{ "title": "A0P0" }]));

    // Under a relationship, loaded ahead with its rows.
    let page = run(&data, r#"{ listAuthors(filter: { name: { eq: "A0" } }) { results { posts(sort: [{ field: TITLE }]) { commentCount } } } }"#, actor()).await;
    assert_eq!(page["listAuthors"]["results"], json!([{ "posts": [{ "commentCount": 3 }, { "commentCount": 2 }] }]));

    let page = run(
        &data,
        &format!(r#"mutation {{ retitlePost(id: "{post}", input: {{ title: "Renamed" }}) {{ result {{ title commentCount }} }} }}"#),
        actor(),
    )
    .await;
    assert_eq!(page["retitlePost"]["result"], json!({ "title": "Renamed", "commentCount": 3 }));
}

/// A field policy hides an aggregate a mutation's result selects, as it does a read's.
#[tokio::test]
async fn a_mutation_result_hides_what_field_policies_hide() {
    let (data, post) = seeded().await;
    let mutation = |title: &str| {
        format!(r#"mutation {{ retitlePost(id: "{post}", input: {{ title: "{title}" }}) {{ result {{ title commentCount }} }} }}"#)
    };
    let page = run(&data, &mutation("Anonymous"), None).await;
    assert_eq!(page["retitlePost"]["result"], json!({ "title": "Anonymous", "commentCount": null }));
    let page = run(&data, &mutation("Signed in"), Some(Actor::new(Uuid::new_v4()))).await;
    assert_eq!(page["retitlePost"]["result"], json!({ "title": "Signed in", "commentCount": 2 }));
}

/// A calculation loaded with the record is its value, nil included: the narrowed record
/// can't compute it again. One only Rust computes reads every attribute it might need.
#[tokio::test]
async fn calculations_load_as_the_record_is_stored() {
    let (data, _) = seeded().await;
    let actor = || Some(Actor::new(Uuid::new_v4()));
    let page = run(&data, r#"{ listPosts(filter: { title: { in: ["A0P0", "A0P1"] } }, sort: [{ field: TITLE }]) { results { firstLabel } } }"#, actor()).await;
    assert_eq!(page["listPosts"]["results"], json!([{ "firstLabel": null }, { "firstLabel": "other" }]));
    data.take_selects();

    let page = run(&data, r#"{ listPosts(filter: { title: { eq: "A1P0" } }) { results { shout } } }"#, actor()).await;
    assert_eq!(page["listPosts"]["results"], json!([{ "shout": "A1P0" }]));
}

static PROJECT_DEF: ResourceDef = ResourceDef {
    name: "Project",
    table: "projects",
    attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
    relationships: &[rel("invoices", RelKind::HasMany, || &INVOICE_DEF, "id", "project_id")],
    aggregates: &[AggregateDef::count("invoice_count", "invoices")],
    ..AUTHOR_DEF
};

/// Each invoice is its owner's to read.
static INVOICE_DEF: ResourceDef = ResourceDef {
    name: "Invoice",
    table: "invoices",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("project_id", AttrType::Uuid),
        AttributeDef::required("owner_id", AttrType::Uuid),
    ],
    policies: &[ash_core::PolicyDef::when(
        ash_core::PolicyWhen::ActionType(ash_core::ActionKind::Read),
        &[PolicyEffect::AuthorizeIf(Check::RelatesToActor { field: "owner_id" })],
    )],
    ..AUTHOR_DEF
};

/// An aggregate counts only the related rows the actor may read, as Ash authorizes an
/// aggregate's query by default: what loading the relationship would return.
#[tokio::test]
async fn an_aggregate_counts_only_what_the_actor_may_read() {
    let data = Counting::default();
    let project = insert(&data, &PROJECT_DEF, &[("name", Value::from("P"))]).await;
    let (mine, theirs) = (Uuid::new_v4(), Uuid::new_v4());
    for owner in [mine, mine, theirs] {
        insert(&data, &INVOICE_DEF, &[("project_id", Value::Uuid(project)), ("owner_id", Value::Uuid(owner))]).await;
    }
    let schema = AshGraphQL::from_resources(&[&PROJECT_DEF, &INVOICE_DEF]).finish::<Counting>().unwrap();
    let res = schema
        .execute(
            Request::new("{ listProjects { results { invoiceCount invoices { id } } } }")
                .data(Context::new(data.clone()).with_actor(Actor::new(mine))),
        )
        .await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let page = res.data.into_json().unwrap();
    let project = &page["listProjects"]["results"][0];
    assert_eq!(project["invoiceCount"], 2);
    assert_eq!(project["invoices"].as_array().unwrap().len(), 2);
}

/// A field policy that checks a field the writer can't read checks it as stored, not as
/// the write's result shows it (hidden, so nil).
#[tokio::test]
async fn a_mutation_result_checks_field_policies_against_the_record_as_stored() {
    let (data, post) = seeded().await;
    data.update(&POST_DEF, None, post, FieldMap::from([("label".to_string(), Value::from("secret"))])).await.unwrap();
    let page = run(
        &data,
        &format!(r#"mutation {{ retitlePost(id: "{post}", input: {{ title: "Labelled" }}) {{ result {{ title commentCount }} }} }}"#),
        Some(Actor::new(Uuid::new_v4())),
    )
    .await;
    assert_eq!(page["retitlePost"]["result"], json!({ "title": "Labelled", "commentCount": null }));
}
