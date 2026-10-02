use ash_core::{AttrType, AttributeDef, Context, DataLayer, FieldMap, ResourceDef, Value};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
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
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

async fn seed_tickets(data: &Memory) {
    for i in 1..=5 {
        let id = Uuid::new_v4();
        let mut map = FieldMap::new();
        map.insert("id".into(), Value::Uuid(id));
        map.insert("title".into(), Value::String(format!("Ticket #{i}")));
        map.insert("priority".into(), Value::Int(i));
        data.create(&TICKET_DEF, None, id, map).await.unwrap();
    }
}

/// Pages a list query through its keyset pages, as AshGraphql's keyset pagination does:
/// `first` / `after` forwards, `last` / `before` backwards.
#[tokio::test]
async fn list_queries_page_by_keyset() {
    let memory = Memory::new();
    seed_tickets(&memory).await;
    let ctx = Context::new(memory);
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let run = |args: String| {
        let schema = schema.clone();
        let ctx = ctx.clone();
        async move {
            let query = format!(
                "{{ listTickets(sort: [{{ field: PRIORITY, order: ASC }}]{args}) {{ count startKeyset endKeyset results {{ priority }} }} }}"
            );
            let res = schema.execute(Request::new(query).data(ctx)).await;
            assert!(res.errors.is_empty(), "{:?}", res.errors);
            res.data.into_json().unwrap()["listTickets"].clone()
        }
    };
    let priorities = |page: &serde_json::Value| -> Vec<i64> {
        page["results"].as_array().unwrap().iter().map(|t| t["priority"].as_i64().unwrap()).collect()
    };

    // Without paging, every record.
    assert_eq!(priorities(&run(String::new()).await), [1, 2, 3, 4, 5]);

    let first = run(", first: 2".into()).await;
    assert_eq!(priorities(&first), [1, 2]);
    assert_eq!(first["count"], 5);
    let after = first["endKeyset"].as_str().unwrap().to_string();

    let second = run(format!(r#", first: 2, after: "{after}""#)).await;
    assert_eq!(priorities(&second), [3, 4]);
    let after = second["endKeyset"].as_str().unwrap().to_string();

    let third = run(format!(r#", first: 2, after: "{after}""#)).await;
    assert_eq!(priorities(&third), [5]);

    // Backwards from the third page's start.
    let before = third["startKeyset"].as_str().unwrap().to_string();
    let back = run(format!(r#", last: 2, before: "{before}""#)).await;
    assert_eq!(priorities(&back), [3, 4]);

    // A page holds at most 250 records, as Ash's default `max_page_size`.
    assert_eq!(priorities(&run(", first: 1000".into()).await).len(), 5);
}

/// A list given no paging arguments doesn't page, as Ash doesn't: it reads every record
/// in the order asked for, with no keysets.
#[tokio::test]
async fn an_unpaged_list_has_no_keysets() {
    let memory = Memory::new();
    seed_tickets(&memory).await;
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");
    let query = "{ listTickets(sort: [{ field: PRIORITY, order: DESC }]) { count startKeyset endKeyset results { priority } } }";
    let res = schema.execute(Request::new(query).data(Context::new(memory))).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    let page = &res.data.into_json().unwrap()["listTickets"];
    assert_eq!(page["count"], 5);
    assert!(page["startKeyset"].is_null() && page["endKeyset"].is_null(), "{page}");
    let priorities: Vec<i64> = page["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["priority"].as_i64().unwrap())
        .collect();
    assert_eq!(priorities, [5, 4, 3, 2, 1]);
}
