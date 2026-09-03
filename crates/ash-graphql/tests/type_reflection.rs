use ash_core::{
    Actor, AttrType, AttributeDef, CalculationDef, Check, Expr, FieldMap, FieldPolicyDef,
    PolicyEffect, ResourceDef, Value,
};
use ash_graphql::AshGraphQL;
use async_graphql::dynamic::*;
use async_graphql::Request;
use uuid::Uuid;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["open", "in_progress", "closed"],
        },
    ),
    AttributeDef::optional("secret_notes", AttrType::String),
];

static TICKET_CALCS: &[CalculationDef] = &[
    CalculationDef::new("upper_title", AttrType::String, Expr::Upper(&Expr::Field("title"))),
];

static TICKET_FIELD_POLICIES: &[FieldPolicyDef] = &[
    FieldPolicyDef {
        field: "secret_notes",
        checks: &[PolicyEffect::AuthorizeIf(Check::ActorPresent)],
    },
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: &[],
    actions: &[],
    policies: &[],
    field_policies: TICKET_FIELD_POLICIES,
    calculations: TICKET_CALCS,
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
async fn test_phase1_type_reflection_and_schema_assembly() {
    let schema = AshGraphQL::from_resources(&[&TICKET_DEF])
        .finish()
        .expect("Failed to build dynamic schema");

    // Introspect the schema to verify Ticket type and enum existence
    let query = r#"
        query {
            __schema {
                types {
                    name
                }
            }
        }
    "#;

    let res = schema.execute(query).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let data_str = res.data.to_string();
    assert!(data_str.contains("Ticket"), "Schema must contain Ticket type");
    assert!(data_str.contains("TicketStatusEnum"), "Schema must contain TicketStatusEnum");
}

#[tokio::test]
async fn test_phase1_field_resolver_and_policy_redaction() {
    // Construct schema with a custom top-level query returning a Ticket
    let mut query = Object::new("Query");
    let ticket_id = Uuid::new_v4();

    query = query.field(Field::new("ticket", TypeRef::named("Ticket"), move |_ctx| {
        let mut map = FieldMap::new();
        map.insert("id".into(), Value::Uuid(ticket_id));
        map.insert("title".into(), Value::String("Fix GraphQL bug".into()));
        map.insert("status".into(), Value::String("open".into()));
        map.insert("secret_notes".into(), Value::String("Top secret diagnostics".into()));
        FieldFuture::new(async move {
            Ok(Some(FieldValue::owned_any(map)))
        })
    }));

    let mut builder = Schema::build("Query", None, None).register(query);
    builder = builder.register(Scalar::new("JSON"));
    builder = builder.register(ash_graphql::build_resource_object(&TICKET_DEF));
    for e in ash_graphql::collect_enums_for_resource(&TICKET_DEF) {
        builder = builder.register(e);
    }
    let schema = builder.finish().expect("Failed to build schema");

    let gql_query = r#"
        query {
            ticket {
                id
                title
                status
                upper_title
                secret_notes
            }
        }
    "#;

    // 1. Unauthenticated request (no actor) -> secret_notes must be redacted (null)
    let res = schema.execute(Request::new(gql_query)).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let ticket_json = &val["ticket"];
    assert_eq!(ticket_json["title"], "Fix GraphQL bug");
    assert_eq!(ticket_json["status"], "OPEN");
    assert_eq!(ticket_json["upper_title"], "FIX GRAPHQL BUG");
    assert!(ticket_json["secret_notes"].is_null(), "secret_notes should be redacted when no actor is present");

    // 2. Authenticated request (actor present) -> secret_notes must be visible
    let actor = Actor::new(Uuid::new_v4());
    let req = Request::new(gql_query).data(actor);
    let res = schema.execute(req).await;
    assert!(res.errors.is_empty(), "Errors: {:?}", res.errors);
    let val = res.data.into_json().unwrap();
    let ticket_json = &val["ticket"];
    assert_eq!(ticket_json["secret_notes"], "Top secret diagnostics");
}
