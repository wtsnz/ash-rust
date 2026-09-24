use ash_core::{
    ActionDef, ArgumentDef, AttrType, AttributeDef, Context, DataLayer, FieldMap, Filter,
    PreparationDef, ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["open", "in_progress", "closed"],
        },
    ),
    AttributeDef::required("is_published", AttrType::Boolean),
];

static TICKET_READ_ACTION: ActionDef = ActionDef::read("read")
    .primary()
    .arguments(&[ArgumentDef::optional("priority", AttrType::Integer)]);

static TICKET_BY_STATUS_ACTION: ActionDef = ActionDef::read("by_status")
    .arguments(&[ArgumentDef::new("status", AttrType::String)])
    .preparations(&[PreparationDef::filter_with_args(|args| {
        if let Some(Value::String(s)) = args.get("status") {
            Filter::eq("status", Value::String(s.clone()))
        } else {
            Filter::True
        }
    })]);

static TICKET_ACTIONS: &[ActionDef] = &[TICKET_READ_ACTION, TICKET_BY_STATUS_ACTION];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: &[],
    actions: TICKET_ACTIONS,
    policies: &[],
    field_policies: &[],
    calculations: &[],
    aggregates: &[],
    extensions: &[],
    notifiers: &[],
    identities: &[],
    indexes: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn seed_data(data: &Memory) -> (Uuid, Uuid, Uuid) {
    let id1 = Uuid::new_v4();
    let mut f1 = FieldMap::new();
    f1.insert("id".into(), Value::Uuid(id1));
    f1.insert("title".into(), Value::String("Fix memory leak".into()));
    f1.insert("priority".into(), Value::Int(1));
    f1.insert("status".into(), Value::String("open".into()));
    f1.insert("is_published".into(), Value::Bool(true));
    data.create(&TICKET_DEF, id1, f1).await.unwrap();

    let id2 = Uuid::new_v4();
    let mut f2 = FieldMap::new();
    f2.insert("id".into(), Value::Uuid(id2));
    f2.insert("title".into(), Value::String("Add dark mode".into()));
    f2.insert("priority".into(), Value::Int(5));
    f2.insert("status".into(), Value::String("in_progress".into()));
    f2.insert("is_published".into(), Value::Bool(false));
    data.create(&TICKET_DEF, id2, f2).await.unwrap();

    let id3 = Uuid::new_v4();
    let mut f3 = FieldMap::new();
    f3.insert("id".into(), Value::Uuid(id3));
    f3.insert("title".into(), Value::String("Improve docs".into()));
    f3.insert("priority".into(), Value::Int(10));
    f3.insert("status".into(), Value::String("closed".into()));
    f3.insert("is_published".into(), Value::Bool(true));
    data.create(&TICKET_DEF, id3, f3).await.unwrap();

    (id1, id2, id3)
}

#[tokio::test]
async fn test_phase2_get_query_by_id() {
    let memory = Memory::new();
    let (id1, _, _) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    let query = format!(
        r#"
        query {{
            getTicket(id: "{}") {{
                id
                title
                priority
                status
                is_published
            }}
        }}
    "#,
        id1
    );

    let res = schema.execute(Request::new(query).data(ctx)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let ticket = &val["getTicket"];
    assert_eq!(ticket["id"], id1.to_string());
    assert_eq!(ticket["title"], "Fix memory leak");
    assert_eq!(ticket["priority"], 1);
    assert_eq!(ticket["status"], "OPEN");
    assert_eq!(ticket["is_published"], true);
}

#[tokio::test]
async fn test_phase2_list_query_with_filters() {
    let memory = Memory::new();
    let (_, _, _) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    // 1. Filter by Enum status == OPEN
    let query_enum = r#"
        query {
            listTickets(filter: { status: { eq: OPEN } }) {
                title
                status
            }
        }
    "#;
    let res = schema.execute(Request::new(query_enum).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["title"], "Fix memory leak");

    // 2. Filter by numeric priority >= 5
    let query_priority = r#"
        query {
            listTickets(filter: { priority: { gte: 5 } }) {
                title
                priority
            }
        }
    "#;
    let res = schema.execute(Request::new(query_priority).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 2);

    // 3. Complex boolean filter: (priority >= 5 AND is_published == true)
    let query_complex = r#"
        query {
            listTickets(filter: {
                and: [
                    { priority: { gte: 5 } },
                    { is_published: { eq: true } }
                ]
            }) {
                title
            }
        }
    "#;
    let res = schema.execute(Request::new(query_complex).data(ctx)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["title"], "Improve docs");
}

#[tokio::test]
async fn test_phase2_list_query_with_sorting_and_pagination() {
    let memory = Memory::new();
    let (_, _, _) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    // Sort by priority descending
    let query_sort = r#"
        query {
            listTickets(sort: [{ field: PRIORITY, order: DESC }]) {
                title
                priority
            }
        }
    "#;
    let res = schema.execute(Request::new(query_sort).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 3);
    assert_eq!(tickets[0]["title"], "Improve docs");
    assert_eq!(tickets[0]["priority"], 10);
    assert_eq!(tickets[1]["title"], "Add dark mode");
    assert_eq!(tickets[1]["priority"], 5);
    assert_eq!(tickets[2]["title"], "Fix memory leak");
    assert_eq!(tickets[2]["priority"], 1);

    // Limit and offset
    let query_limit_offset = r#"
        query {
            listTickets(
                sort: [{ field: PRIORITY, order: DESC }],
                limit: 1,
                offset: 1
            ) {
                title
                priority
            }
        }
    "#;
    let res = schema.execute(Request::new(query_limit_offset).data(ctx)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["title"], "Add dark mode");
    assert_eq!(tickets[0]["priority"], 5);
}

#[tokio::test]
async fn test_phase2_read_query_with_action_arguments() {
    let memory = Memory::new();
    let (_id1, _id2, _id3) = seed_data(&memory).await;
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    // 1. Primary read query with argument (listTickets(priority: 5))
    let query_with_arg = r#"
        query {
            listTickets(priority: 5) {
                title
                priority
            }
        }
    "#;
    let res = schema.execute(Request::new(query_with_arg).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["listTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["title"], "Add dark mode");
    assert_eq!(tickets[0]["priority"], 5);

    // 2. Custom read action query (byStatusTickets(status: "closed"))
    let query_custom_action = r#"
        query {
            byStatusTickets(status: "closed") {
                title
                status
            }
        }
    "#;
    let res = schema.execute(Request::new(query_custom_action).data(ctx)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let tickets = val["byStatusTickets"].as_array().unwrap();
    assert_eq!(tickets.len(), 1);
    assert_eq!(tickets[0]["title"], "Improve docs");
    assert_eq!(tickets[0]["status"], "CLOSED");
}

