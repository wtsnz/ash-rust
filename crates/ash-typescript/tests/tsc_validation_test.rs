use std::fs;
use std::process::Command;
use ash_core::{ActionDef, AttrType, AttributeDef, DomainDef, RelationshipDef, ResourceDef};
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use uuid::Uuid;

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
    multitenancy: None,
};

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::Atom { one_of: &["open", "closed"] }),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("author_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[
    RelationshipDef::belongs_to("author", || &USER_DEST, "author_id"),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open").accept(&["title", "status", "priority", "author_id"]),
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
    multitenancy: None,
};

static DOMAIN: DomainDef = DomainDef {
    name: "Helpdesk",
    resources: &[&TICKET_DEF, &USER_DEST],
};

#[test]
fn test_typescript_syntax_validity_with_tsc() {
    // Generate TypeScript without external deps (no zod/react import required for raw typecheck)
    let generator = TypeScriptGenerator::new()
        .config(
            TypeScriptConfig::new()
                .with_zod(false)
                .with_client(true)
                .with_react(false)
                .with_client_name("HelpdeskClient"),
        )
        .add_domain(&DOMAIN);

    let ts_code = generator.generate_consolidated().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("ash_ts_test_{}", Uuid::new_v4().simple()));
    fs::create_dir_all(&temp_dir).unwrap();
    let file_path = temp_dir.join("test_sdk.ts");
    fs::write(&file_path, ts_code).unwrap();

    // Check with tsc if installed
    if let Ok(output) = Command::new("tsc")
        .arg("--noEmit")
        .arg("--target")
        .arg("ES2022")
        .arg("--moduleResolution")
        .arg("node")
        .arg("--skipLibCheck")
        .arg(&file_path)
        .output()
    {
        assert!(
            output.status.success(),
            "tsc validation failed!\nSTDOUT:\n{}\nSTDERR:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let _ = fs::remove_dir_all(&temp_dir);
}
