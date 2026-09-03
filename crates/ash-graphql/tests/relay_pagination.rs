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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

async fn seed_tickets(data: &Memory) {
    for i in 1..=5 {
        let id = Uuid::new_v4();
        let mut map = FieldMap::new();
        map.insert("id".into(), Value::Uuid(id));
        map.insert("title".into(), Value::String(format!("Ticket #{i}")));
        map.insert("priority".into(), Value::Int(i));
        data.create(&TICKET_DEF, id, map).await.unwrap();
    }
}

#[tokio::test]
async fn test_phase3_relay_keyset_cursor_pagination() {
    let memory = Memory::new();
    seed_tickets(&memory).await;
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    // 1. Page 1: first 2 items sorted by priority ascending
    let query_page1 = r#"
        query {
            ticketsConnection(first: 2, sort: [{ field: PRIORITY, order: ASC }]) {
                totalCount
                pageInfo {
                    hasNextPage
                    hasPreviousPage
                    startCursor
                    endCursor
                }
                edges {
                    cursor
                    node {
                        title
                        priority
                    }
                }
            }
        }
    "#;

    let res1 = schema.execute(Request::new(query_page1).data(ctx.clone())).await;
    assert!(res1.errors.is_empty(), "Page 1 errors: {:?}", res1.errors);
    let val1 = res1.data.into_json().unwrap();
    let conn1 = &val1["ticketsConnection"];

    assert_eq!(conn1["totalCount"], 5);
    let edges1 = conn1["edges"].as_array().unwrap();
    assert_eq!(edges1.len(), 2);
    assert_eq!(edges1[0]["node"]["priority"], 1);
    assert_eq!(edges1[1]["node"]["priority"], 2);

    let page_info1 = &conn1["pageInfo"];
    assert_eq!(page_info1["hasNextPage"], true);
    assert_eq!(page_info1["hasPreviousPage"], false);
    let end_cursor1 = page_info1["endCursor"].as_str().unwrap().to_string();

    // 2. Page 2: next 2 items after end_cursor1
    let query_page2 = format!(
        r#"
        query {{
            ticketsConnection(first: 2, after: "{}", sort: [{{ field: PRIORITY, order: ASC }}]) {{
                pageInfo {{
                    hasNextPage
                    hasPreviousPage
                    endCursor
                }}
                edges {{
                    node {{
                        title
                        priority
                    }}
                }}
            }}
        }}
    "#,
        end_cursor1
    );

    let res2 = schema.execute(Request::new(query_page2).data(ctx.clone())).await;
    assert!(res2.errors.is_empty(), "Page 2 errors: {:?}", res2.errors);
    let val2 = res2.data.into_json().unwrap();
    let conn2 = &val2["ticketsConnection"];
    let edges2 = conn2["edges"].as_array().unwrap();
    assert_eq!(edges2.len(), 2);
    assert_eq!(edges2[0]["node"]["priority"], 3);
    assert_eq!(edges2[1]["node"]["priority"], 4);

    let page_info2 = &conn2["pageInfo"];
    assert_eq!(page_info2["hasNextPage"], true);
    assert_eq!(page_info2["hasPreviousPage"], true);
    let end_cursor2 = page_info2["endCursor"].as_str().unwrap().to_string();

    // 3. Page 3: last remaining item after end_cursor2
    let query_page3 = format!(
        r#"
        query {{
            ticketsConnection(first: 2, after: "{}", sort: [{{ field: PRIORITY, order: ASC }}]) {{
                pageInfo {{
                    hasNextPage
                    hasPreviousPage
                }}
                edges {{
                    node {{
                        title
                        priority
                    }}
                }}
            }}
        }}
    "#,
        end_cursor2
    );

    let res3 = schema.execute(Request::new(query_page3).data(ctx)).await;
    assert!(res3.errors.is_empty(), "Page 3 errors: {:?}", res3.errors);
    let val3 = res3.data.into_json().unwrap();
    let conn3 = &val3["ticketsConnection"];
    let edges3 = conn3["edges"].as_array().unwrap();
    assert_eq!(edges3.len(), 1);
    assert_eq!(edges3[0]["node"]["priority"], 5);

    let page_info3 = &conn3["pageInfo"];
    assert_eq!(page_info3["hasNextPage"], false);
    assert_eq!(page_info3["hasPreviousPage"], true);
}
