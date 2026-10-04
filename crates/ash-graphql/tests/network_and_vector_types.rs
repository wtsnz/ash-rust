use ash_core::{ActionDef, AttrType, AttributeDef, Context, ResourceDef};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::json;

static DEVICE_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("address", AttrType::Inet),
    AttributeDef::optional("embedding", AttrType::Vector { dimensions: 3 }),
    AttributeDef::optional("weight", AttrType::Float),
];

static DEVICE_ACTIONS: &[ActionDef] = &[
    ActionDef::create("create").accept(&["address", "embedding", "weight"]),
    ActionDef::read("read").primary().pagination(ash_core::Pagination::keyset().countable(ash_core::Countable::Yes).required(false)),
];

static DEVICE_DEF: ResourceDef = ResourceDef {
    name: "Device",
    table: "devices",
    attributes: DEVICE_ATTRS,
    relationships: &[],
    actions: DEVICE_ACTIONS,
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
async fn inet_and_vector_round_trip_through_graphql() {
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&DEVICE_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    let create = r#"
        mutation {
            createDevice(input: { address: "10.0.0.1/32", embedding: [0.1, 2.5, -3] }) {
                errors { code }
                result { address embedding }
            }
        }
    "#;
    let res = schema.execute(Request::new(create).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(
        res.data.into_json().unwrap()["createDevice"],
        json!({
            "errors": [],
            "result": { "address": "10.0.0.1", "embedding": [0.1, 2.5, -3.0] }
        })
    );

    let list = r#"
        query {
            listDevices(filter: { address: { eq: "10.0.0.1/32" } }) { results { address embedding } }
        }
    "#;
    let res = schema.execute(Request::new(list).data(ctx.clone())).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(
        res.data.into_json().unwrap()["listDevices"]["results"],
        json!([{ "address": "10.0.0.1", "embedding": [0.1, 2.5, -3.0] }])
    );

    for (input, message) in [
        (r#"{ address: "10.0.0.300" }"#, "invalid inet"),
        (r#"{ address: "10.0.0.1", embedding: [1, 2] }"#, "3 dimensions"),
    ] {
        let mutation = format!("mutation {{ createDevice(input: {input}) {{ errors {{ code }} }} }}");
        let res = schema.execute(Request::new(mutation).data(ctx.clone())).await;
        let errors = format!("{:?}", res.errors);
        assert!(errors.contains(message), "{input}: {errors}");
    }
}

#[tokio::test]
async fn float_attributes_are_numbers_in_graphql() {
    let ctx = Context::new(Memory::new());
    let schema = AshGraphQL::from_resources(&[&DEVICE_DEF])
        .finish::<Memory>()
        .expect("Failed to build schema");

    let create = r#"
        mutation {
            createDevice(input: { address: "10.0.0.1", weight: 2.5 }) { result { weight } }
        }
    "#;
    let res = schema.execute(Request::new(create).data(ctx)).await;
    assert!(res.errors.is_empty(), "{:?}", res.errors);
    assert_eq!(
        res.data.into_json().unwrap()["createDevice"]["result"]["weight"],
        json!(2.5)
    );
}
