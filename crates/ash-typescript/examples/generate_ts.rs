use ash_core::{ActionDef, AttrType, AttributeDef, DomainDef, RelationshipDef, ResourceDef, Validation};
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};

static USER_DEST: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("name", AttrType::String),
        AttributeDef::required("email", AttrType::String),
    ],
    relationships: &[],
    actions: &[
        ActionDef::create("create").accept(&["name", "email"]),
    ],
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
    store_type_id: || std::any::TypeId::of::<()>(),
    store_name: "memory",
};

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::Atom { one_of: &["open", "in_progress", "closed"] }),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("author_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[
    RelationshipDef::belongs_to("author", || &USER_DEST, "author_id"),
];

static TICKET_VALIDATIONS: &[Validation] = &[
    Validation::present("title"),
    Validation::string_length("title", Some(5), Some(255)),
    Validation::numericality("priority", Some(1), Some(5)),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["title", "status", "priority", "author_id"])
        .validations(TICKET_VALIDATIONS),
    ActionDef::update("close").accept(&["status"]),
    ActionDef::destroy("destroy"),
];

static TICKET_DEF: ResourceDef = ResourceDef {
    name: "Ticket",
    table: "tickets",
    attributes: TICKET_ATTRS,
    relationships: TICKET_RELS,
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
    store_type_id: || std::any::TypeId::of::<()>(),
    store_name: "memory",
};

static DOMAIN: DomainDef = DomainDef {
    name: "Helpdesk",
    resources: &[&TICKET_DEF, &USER_DEST],
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Generating TypeScript SDK for Helpdesk Domain...");

    let generator = TypeScriptGenerator::new()
        .config(
            TypeScriptConfig::new()
                .with_zod(true)
                .with_client(true)
                .with_react(true)
                .with_client_name("HelpdeskClient")
                .with_graphql_endpoint("/graphql"),
        )
        .add_domain(&DOMAIN);

    let ts_code = generator.generate_consolidated()?;

    println!("------------------------------------------------------------");
    println!("Generated {} lines of TypeScript code.", ts_code.lines().count());
    println!("Sample snippet:\n{}", &ts_code[..ts_code.find("// --- Section 2").unwrap_or(500)]);
    println!("------------------------------------------------------------");

    Ok(())
}
