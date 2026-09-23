use ash_core::{ActionDef, AttrType, AttributeDef, RelationshipDef, ResourceDef};
use ash_typescript::types::*;

static USER_DEST: ResourceDef = ResourceDef {
    name: "User",
    table: "users",
    attributes: &[AttributeDef::uuid_pk("id")],
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
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: || std::any::TypeId::of::<()>(),
    store_name: "memory",
    multitenancy: None,
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

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["title", "status", "priority", "author_id"]),
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
    indexes: &[],
    checks: &[],
    embedded: false,
    data_layer: ash_core::DataLayerKind::Memory,
    timestamps: None,
    store_type_id: || std::any::TypeId::of::<()>(),
    store_name: "memory",
    multitenancy: None,
};

#[test]
fn test_resource_interface_generation() {
    let ts = generate_resource_interface(&TICKET_DEF);
    assert!(ts.contains("export interface Ticket {"));
    assert!(ts.contains("  id: string;"));
    assert!(ts.contains("  title: string;"));
    assert!(ts.contains("  status: \"open\" | \"in_progress\" | \"closed\";"));
    assert!(ts.contains("  priority: number;"));
    assert!(ts.contains("  author_id?: string | null;"));
    assert!(ts.contains("  author?: User | null;"));
}

#[test]
fn test_action_input_interface() {
    let ts = generate_action_input_interface(&TICKET_DEF, "open").unwrap();
    assert!(ts.contains("export interface OpenTicketInput {"));
    assert!(ts.contains("export type TicketOpenInput = OpenTicketInput;"));
    assert!(ts.contains("  title: string;"));
    assert!(ts.contains("  status: \"open\" | \"in_progress\" | \"closed\";"));
    assert!(ts.contains("  priority: number;"));
    assert!(ts.contains("  author_id?: string | null;"));
}

#[test]
fn test_filter_and_sort_generation() {
    let filter = generate_resource_filter_input(&TICKET_DEF);
    assert!(filter.contains("export interface TicketFilterInput {"));
    assert!(filter.contains("  id?: UuidFilter;"));
    assert!(filter.contains("  title?: StringFilter;"));
    assert!(filter.contains("  priority?: IntFilter;"));
    assert!(filter.contains("  author?: UserFilterInput;"));
    assert!(filter.contains("  and?: TicketFilterInput[];"));

    let sort = generate_resource_sort_input(&TICKET_DEF);
    assert!(sort.contains("export type TicketSortField = \"id\" | \"title\" | \"status\" | \"priority\" | \"author_id\";"));
    assert!(sort.contains("export interface TicketSortInput {"));
}

#[test]
fn test_has_one_typescript_interface() {
    static PROFILE_ATTRS: &[AttributeDef] = &[
        AttributeDef::uuid_pk("id"),
        AttributeDef::required("bio", AttrType::String),
    ];
    static PROFILE_DEF: ResourceDef = ResourceDef {
        name: "Profile",
        table: "profiles",
        attributes: PROFILE_ATTRS,
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
        embedded: false,
        data_layer: ash_core::DataLayerKind::Memory,
        timestamps: None,
        store_type_id: || std::any::TypeId::of::<()>(),
        store_name: "memory",
        multitenancy: None,
    };

    static USER_RELS: &[RelationshipDef] = &[
        RelationshipDef::has_one("profile", || &PROFILE_DEF, "user_id"),
    ];

    static USER_DEF_HAS_ONE: ResourceDef = ResourceDef {
        name: "User",
        table: "users",
        attributes: &[AttributeDef::uuid_pk("id"), AttributeDef::required("name", AttrType::String)],
        relationships: USER_RELS,
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
        store_type_id: || std::any::TypeId::of::<()>(),
        store_name: "memory",
        multitenancy: None,
    };

    let ts = generate_resource_interface(&USER_DEF_HAS_ONE);
    assert!(ts.contains("  profile?: Profile | null;"));

    let filter = generate_resource_filter_input(&USER_DEF_HAS_ONE);
    assert!(filter.contains("  profile?: ProfileFilterInput;"));
}
