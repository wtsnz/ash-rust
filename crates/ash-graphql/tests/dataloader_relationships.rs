use ash_core::{
    AttrType, AttributeDef, Context, DataLayer, FieldMap, OnDelete, RelKind, RelationshipDef,
    ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use uuid::Uuid;

static AUTHOR_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
];

static AUTHOR_RELS: &[RelationshipDef] = &[RelationshipDef {
    name: "posts",
    kind: RelKind::HasMany,
    destination: || &POST_DEF,
    source_attribute: "id",
    destination_attribute: "author_id",
    through: None,
    source_attribute_on_join_resource: None,
    destination_attribute_on_join_resource: None,
    on_delete: OnDelete::Cascade,
}];

static AUTHOR_DEF: ResourceDef = ResourceDef {
    name: "Author",
    table: "authors",
    attributes: AUTHOR_ATTRS,
    relationships: AUTHOR_RELS,
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

static POST_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("author_id", AttrType::Uuid),
];

static POST_RELS: &[RelationshipDef] = &[RelationshipDef {
    name: "author",
    kind: RelKind::BelongsTo,
    destination: || &AUTHOR_DEF,
    source_attribute: "author_id",
    destination_attribute: "id",
    through: None,
    source_attribute_on_join_resource: None,
    destination_attribute_on_join_resource: None,
    on_delete: OnDelete::Nothing,
}];

static POST_DEF: ResourceDef = ResourceDef {
    name: "Post",
    table: "posts",
    attributes: POST_ATTRS,
    relationships: POST_RELS,
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn seed_data(data: &Memory) -> (Uuid, Uuid) {
    let author1_id = Uuid::new_v4();
    let mut a1 = FieldMap::new();
    a1.insert("id".into(), Value::Uuid(author1_id));
    a1.insert("name".into(), Value::String("Alice".into()));
    data.create(&AUTHOR_DEF, author1_id, a1).await.unwrap();

    let author2_id = Uuid::new_v4();
    let mut a2 = FieldMap::new();
    a2.insert("id".into(), Value::Uuid(author2_id));
    a2.insert("name".into(), Value::String("Bob".into()));
    data.create(&AUTHOR_DEF, author2_id, a2).await.unwrap();

    // Alice's posts
    for i in 1..=2 {
        let p_id = Uuid::new_v4();
        let mut p = FieldMap::new();
        p.insert("id".into(), Value::Uuid(p_id));
        p.insert("title".into(), Value::String(format!("Alice Post #{i}")));
        p.insert("author_id".into(), Value::Uuid(author1_id));
        data.create(&POST_DEF, p_id, p).await.unwrap();
    }

    // Bob's posts
    for i in 1..=2 {
        let p_id = Uuid::new_v4();
        let mut p = FieldMap::new();
        p.insert("id".into(), Value::Uuid(p_id));
        p.insert("title".into(), Value::String(format!("Bob Post #{i}")));
        p.insert("author_id".into(), Value::Uuid(author2_id));
        data.create(&POST_DEF, p_id, p).await.unwrap();
    }

    (author1_id, author2_id)
}

#[tokio::test]
async fn test_phase5_dataloader_belongs_to_and_has_many() {
    let memory = Memory::new();
    let (_alice_id, _bob_id) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let resources = &[&AUTHOR_DEF, &POST_DEF];
    let schema = AshGraphQL::from_resources(resources)
        .finish::<Memory>()
        .expect("Failed to build schema");

    let query_authors = r#"
        query {
            listAuthors {
                name
                posts {
                    title
                }
            }
        }
    "#;

    let dataloader1 = AshGraphQL::create_dataloader(ctx.clone(), resources);
    let req = Request::new(query_authors).data(ctx.clone()).data(dataloader1);
    let res = schema.execute(req).await;
    assert!(res.errors.is_empty(), "Errors in listAuthors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let authors = val["listAuthors"].as_array().unwrap();
    assert_eq!(authors.len(), 2);

    for author in authors {
        let name = author["name"].as_str().unwrap();
        let posts = author["posts"].as_array().unwrap();
        assert_eq!(posts.len(), 2);
        for p in posts {
            let title = p["title"].as_str().unwrap();
            assert!(title.starts_with(name), "Post title should match author name");
        }
    }

    // 2. Query belongs_to relationship: listPosts { title, author { name } }
    let query_posts = r#"
        query {
            listPosts {
                title
                author {
                    name
                }
            }
        }
    "#;

    let dataloader2 = AshGraphQL::create_dataloader(ctx.clone(), resources);
    let req = Request::new(query_posts).data(ctx).data(dataloader2);
    let res = schema.execute(req).await;
    assert!(res.errors.is_empty(), "Errors in listPosts: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let posts = val["listPosts"].as_array().unwrap();
    assert_eq!(posts.len(), 4);

    for post in posts {
        let title = post["title"].as_str().unwrap();
        let author_name = post["author"]["name"].as_str().unwrap();
        assert!(title.starts_with(author_name), "Post author should match title");
    }
}

#[tokio::test]
async fn test_nested_relationship_filtering() {
    let memory = Memory::new();
    let (_alice_id, _bob_id) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let resources = &[&AUTHOR_DEF, &POST_DEF];
    let schema = AshGraphQL::from_resources(resources)
        .finish::<Memory>()
        .expect("Failed to build schema");

    // Filter Posts by related Author name == "Alice"
    let query_posts_by_author = r#"
        query {
            listPosts(filter: { author: { name: { eq: "Alice" } } }) {
                title
                author {
                    name
                }
            }
        }
    "#;

    let res = schema.execute(Request::new(query_posts_by_author).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let posts = val["listPosts"].as_array().unwrap();
    assert_eq!(posts.len(), 2);
    for post in posts {
        assert_eq!(post["author"]["name"], "Alice");
    }

    // Filter Authors by related Posts title == "Bob Post #1"
    let query_authors_by_post = r#"
        query {
            listAuthors(filter: { posts: { title: { eq: "Bob Post #1" } } }) {
                name
            }
        }
    "#;

    let res = schema.execute(Request::new(query_authors_by_post).data(ctx)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let authors = val["listAuthors"].as_array().unwrap();
    assert_eq!(authors.len(), 1);
    assert_eq!(authors[0]["name"], "Bob");
}

