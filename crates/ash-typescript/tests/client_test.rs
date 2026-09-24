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

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::required("status", AttrType::String),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("author_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[
    RelationshipDef::belongs_to("author", || &USER_DEST, "author_id"),
];

static TICKET_VALIDATIONS: &[Validation] = &[
    Validation::present("title"),
    Validation::string_length("title", Some(5), Some(255)),
    Validation::one_of("status", &["open", "closed"]),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::create("open")
        .accept(&["title", "status", "priority", "author_id"])
        .validations(TICKET_VALIDATIONS),
    ActionDef::update("close")
        .accept(&["status"]),
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

static HELPDESK_DOMAIN: DomainDef = DomainDef {
    name: "Helpdesk",
    resources: &[&TICKET_DEF, &USER_DEST],
};

#[test]
fn test_consolidated_code_generation() {
    let generator = TypeScriptGenerator::new()
        .config(
            TypeScriptConfig::new()
                .with_zod(true)
                .with_client(true)
                .with_react(true)
                .with_client_name("HelpdeskClient")
                .with_graphql_endpoint("/api/graphql"),
        )
        .add_domain(&HELPDESK_DOMAIN);

    let ts = generator.generate_consolidated().unwrap();

    // Check imports
    assert!(ts.contains("import { z } from \"zod\";"));

    // Check common types
    assert!(ts.contains("export interface PageInfo {"));
    assert!(ts.contains("export interface UuidFilter {"));

    // Check resource interfaces
    assert!(ts.contains("export interface Ticket {"));
    assert!(ts.contains("export interface User {"));

    // Check Zod validation schemas
    assert!(ts.contains("export const OpenTicketInputSchema = z.object({"));
    assert!(ts.contains("export const TicketOpenInputSchema = OpenTicketInputSchema;"));
    assert!(ts.contains("export const TicketSchema = z.object({"));
    assert!(ts.contains("export const UserSchema = z.object({"));

    // Check Transport and Config
    assert!(ts.contains("export class AshTransport {"));
    assert!(ts.contains("private readonly defaultEndpoint = \"/api/graphql\";"));

    // Check Selection sets
    assert!(ts.contains("export function buildTicketSelectionSet(include?: TicketInclude): string {"));
    assert!(ts.contains("fields += ` author { ${buildUserSelectionSet(subInclude)} }`;"));

    // Check Query Builder
    assert!(ts.contains("export class TicketQueryBuilder {"));
    assert!(ts.contains("public async all(): Promise<Ticket[]> {"));
    assert!(ts.contains("public async first(): Promise<Ticket | null> {"));
    assert!(ts.contains("public async page(first: number = 20, after?: string): Promise<PaginatedResult<Ticket>> {"));
    assert!(ts.contains("public queryOptions() {"));

    // Check Resource Client
    assert!(ts.contains("export class TicketClient {"));
    assert!(ts.contains("public async get(id: string, include?: TicketInclude): Promise<Ticket | null> {"));
    assert!(ts.contains("public async open(input: OpenTicketInput, include?: TicketInclude): Promise<Ticket> {"));
    assert!(ts.contains("public async close(id: string, input: CloseTicketInput, include?: TicketInclude): Promise<Ticket> {"));
    assert!(ts.contains("public async destroy(id: string): Promise<boolean> {"));

    // Check Domain & Root Client
    assert!(ts.contains("export class HelpdeskDomainClient {"));
    assert!(ts.contains("public readonly ticket: TicketClient;"));
    assert!(ts.contains("public readonly user: UserClient;"));

    assert!(ts.contains("export class HelpdeskClient {"));
    assert!(ts.contains("export function createHelpdeskClient(config: AshClientConfig): HelpdeskClient {"));

    // Check React helpers
    assert!(ts.contains("export function useTicketQuery("));
    assert!(ts.contains("export function useOpenTicketMutation("));
}

#[test]
fn test_modular_bundle_generation() {
    let generator = TypeScriptGenerator::new()
        .config(TypeScriptConfig::new().with_client_name("AshClient"))
        .add_resource(&TICKET_DEF);

    let bundle = generator.generate_bundle().unwrap();

    assert!(bundle.types_ts.contains("export interface Ticket {"));
    assert!(bundle.zod_ts.as_ref().unwrap().contains("export const TicketOpenInputSchema"));
    assert!(bundle.client_ts.as_ref().unwrap().contains("export class AshClient {"));
    assert!(bundle.index_ts.contains("export * from \"./types\";"));
    assert!(bundle.index_ts.contains("export * from \"./zod\";"));
    assert!(bundle.index_ts.contains("export * from \"./client\";"));
}
