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
        },
    ),
    AttributeDef {
        name: "version",
        ty: AttrType::Integer,
        primary_key: false,
        allow_nil: false,
        generated: false,
        version: true,
        default_fn: None,
    },
];

static CLOSE_ARGS: &[ArgumentDef] = &[ArgumentDef::optional("reason", AttrType::String)];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["title"]),
    ActionDef::update("close")
        .accept(&["status"])
        .arguments(CLOSE_ARGS),
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
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
                success
                errors {
                    code
                    message
                    field
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
    assert_eq!(create_payload["success"], true);
    assert_eq!(create_payload["errors"].as_array().unwrap().len(), 0);

    let ticket = &create_payload["result"];
    let ticket_id = ticket["id"].as_str().unwrap().to_string();
    assert_eq!(ticket["title"], "Implement Phase 4 Mutations");
    assert_eq!(ticket["version"], 1);

    // 2. Validation failure: missing required field
    let invalid_create = r#"
        mutation {
            createTicket(input: {}) {
                success
                errors {
                    code
                    field
                }
                result {
                    id
                }
            }
        }
    "#;
    let res = schema.execute(Request::new(invalid_create).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let invalid_payload = &val["createTicket"];
    assert_eq!(invalid_payload["success"], false);
    let errors = invalid_payload["errors"].as_array().unwrap();
    assert!(!errors.is_empty());
    assert_eq!(errors[0]["code"], "REQUIRED_FIELD_MISSING");
    assert_eq!(errors[0]["field"], "title");

    // 3. Update ticket with optimistic locking and action arguments
    let update_mutation = format!(
        r#"
        mutation {{
            closeTicket(input: {{
                id: "{ticket_id}",
                status: CLOSED,
                reason: "All tests passing",
                version: 1
            }}) {{
                success
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
    assert_eq!(update_payload["success"], true);
    assert_eq!(update_payload["result"]["status"], "CLOSED");
    assert_eq!(update_payload["result"]["version"], 2);

    // 4. Stale record failure: sending version: 1 when version is now 2
    let stale_update = format!(
        r#"
        mutation {{
            closeTicket(input: {{
                id: "{ticket_id}",
                status: OPEN,
                version: 1
            }}) {{
                success
                errors {{
                    code
                    field
                }}
                result {{
                    id
                }}
            }}
        }}
    "#
    );
    let res = schema.execute(Request::new(stale_update).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let stale_payload = &val["closeTicket"];
    assert_eq!(stale_payload["success"], false);
    let errors = stale_payload["errors"].as_array().unwrap();
    assert_eq!(errors[0]["code"], "STALE_RECORD");
    assert_eq!(errors[0]["field"], "version");

    // 5. Destroy ticket mutation
    let destroy_mutation = format!(
        r#"
        mutation {{
            destroyTicket(input: {{ id: "{ticket_id}", version: 2 }}) {{
                success
                errors {{
                    code
                }}
            }}
        }}
    "#
    );
    let res = schema.execute(Request::new(destroy_mutation).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "GraphQL errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let destroy_payload = &val["destroyTicket"];
    assert_eq!(destroy_payload["success"], true);

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
