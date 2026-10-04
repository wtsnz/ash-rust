use ash_core::{
    ActionDef, ArgumentDef, AttrType, AttributeDef, Context, ResourceDef,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::optional(
        "status",
        AttrType::Atom {
            one_of: &["open", "closed"],
            name: Some("TicketStatus"),
        },
    ),
    AttributeDef {
        name: "version",
        ty: AttrType::Integer,
        primary_key: false,
        allow_nil: false,
        generated: false,
        default_fn: Some(first_version),
    },
];

fn first_version() -> ash_core::Value {
    ash_core::Value::Int(1)
}

static CLOSE_ARGS: &[ArgumentDef] = &[ArgumentDef::optional("reason", AttrType::String)];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["title"]),
    ActionDef::update("close")
        .accept(&["status"])
        .arguments(CLOSE_ARGS)
        .changes(&[ash_core::Change::OptimisticLock { field: "version" }]),
    ActionDef::destroy("destroy"),
];

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
    checks: &[],
    statements: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
    multitenancy: None,
};

#[tokio::test]
async fn test_phase4_create_update_destroy_mutations_and_errors() {
    let memory = Memory::new();
    let ctx = Context::new(memory);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    // 1. Create ticket mutation
    let create_mutation = r#"
        mutation {
            createTicket(input: { title: "Implement Phase 4 Mutations" }) {
                errors {
                    code
                    message
                    fields
                }
                result {
                    id
                    title
                    version
                }
            }
        }
    "#;

    let res = schema.execute(Request::new(create_mutation).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let create_payload = &val["createTicket"];
    assert_eq!(create_payload["errors"].as_array().unwrap().len(), 0);

    let ticket = &create_payload["result"];
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    assert_eq!(ticket["title"], "Implement Phase 4 Mutations");
    assert_eq!(ticket["version"], 1);

    // 2. A required field left out: the input type requires it, as AshGraphql's does.
    let invalid_create = r#"
        mutation {
            createTicket(input: {}) {
                errors { code }
            }
        }
    "#;
    let res = schema.execute(Request::new(invalid_create).data(ctx.clone())).await;
    assert_eq!(res.errors.len(), 1, "{:?}", res.errors);
    assert!(res.errors[0].message.contains("title"), "{:?}", res.errors);

    // 3. Update ticket with optimistic locking and action arguments
    let update_mutation = format!(
        r#"
        mutation {{
            closeTicket(id: "{ticket_id}", input: {{
                status: CLOSED,
                reason: "All tests passing"
            }}) {{
                errors {{
                    code
                    message
                }}
                result {{
                    id
                    status
                    version
                }}
            }}
        }}
    "#
    );

    let res = schema.execute(Request::new(update_mutation).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let update_payload = &val["closeTicket"];
    assert_eq!(update_payload["errors"].as_array().unwrap().len(), 0);
    assert_eq!(update_payload["result"]["status"], "CLOSED");
    assert_eq!(update_payload["result"]["version"], 2);

    // 4. The lock checks the version the record was read at, as AshGraphql's does: a
    // client sends no version of its own.
    let sdl = schema.sdl();
    let input = sdl.split("input CloseTicketInput {").nth(1).and_then(|rest| rest.split('}').next()).unwrap();
    assert!(!input.contains("version"), "{input}");

    // 5. Destroy ticket mutation
    let destroy_mutation = format!(
        r#"
        mutation {{
            destroyTicket(id: "{ticket_id}") {{
                errors {{
                    code
                }}
                result {{ id }}
            }}
        }}
    "#
    );
    let res = schema.execute(Request::new(destroy_mutation).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let destroy_payload = &val["destroyTicket"];
    assert_eq!(destroy_payload["errors"].as_array().unwrap().len(), 0);
    // A destroy's result is the record it destroyed.
    assert_eq!(destroy_payload["result"]["id"], ticket_id);

    // Verify record is destroyed
    let verify_query = format!(
        r#"
        query {{
            getTicket(id: "{ticket_id}") {{
                id
            }}
        }}
    "#
    );
    let res = schema.execute(Request::new(verify_query).data(ctx)).await;
    assert!(res.errors.is_empty());
    let val = res.data.into_json().unwrap();
    assert!(val["getTicket"].is_null());
}
