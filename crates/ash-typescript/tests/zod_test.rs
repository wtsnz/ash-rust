use ash_core::{ActionDef, AttrType, AttributeDef, ResourceDef, Validation};
use ash_typescript::zod::*;

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("author_id", AttrType::Uuid),
];

static TICKET_VALIDATIONS: &[Validation] = &[
    Validation::present("title"),
    Validation::string_length("title", Some(5), Some(255)),
    Validation::one_of("status", &["open", "in_progress", "closed"]),
    Validation::numericality("priority", Some(1), Some(5)),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["title", "status", "priority", "author_id"])
        .validations(TICKET_VALIDATIONS),
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
    store_type_id: || std::any::TypeId::of::<()>(),
    store_name: "memory",
    multitenancy: None,
};

#[test]
fn test_zod_schema_generation() {
    let zod = generate_resource_zod(&TICKET_DEF);
    // Resource schema
    assert!(zod.contains("export const TicketSchema = z.object({"));
    assert!(zod.contains("id: z.string().uuid()"));
    assert!(zod.contains("author_id: z.string().uuid().nullable().optional()"));

    // Action input schema
    assert!(zod.contains("export const OpenTicketInputSchema = z.object({"));
    assert!(zod.contains("export const TicketOpenInputSchema = OpenTicketInputSchema;"));
    assert!(zod.contains("title: z.string().min(5).max(255)"));
    assert!(zod.contains("status: z.enum([\"open\", \"in_progress\", \"closed\"])"));
    assert!(zod.contains("priority: z.number().int().min(1).max(5)"));
    assert!(zod.contains("author_id: z.string().uuid().nullable().optional()"));
}
