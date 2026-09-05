use std::path::Path;
use ash_core::{
    ActionDef, AttrType, AttributeDef, Context, DataLayer, DomainDef, FieldMap,
    RelationshipDef, ResourceDef, Validation, Value,
};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::PubSub;
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use axum::extract::Request;
use axum::http::{header, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use uuid::Uuid;

// --- Resource Definitions ---

static REP_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("name", AttrType::String),
    AttributeDef::required("email", AttrType::String),
    AttributeDef::required("role", AttrType::String),
];

static REP_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("create")
        .primary()
        .accept(&["name", "email", "role"]),
];

pub static REP_DEF: ResourceDef = ResourceDef {
    name: "Representative",
    table: "representatives",
    attributes: REP_ATTRS,
    relationships: &[],
    actions: REP_ACTIONS,
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

static TICKET_ATTRS: &[AttributeDef] = &[
    AttributeDef::uuid_pk("id"),
    AttributeDef::required("title", AttrType::String),
    AttributeDef::optional("description", AttrType::String),
    AttributeDef::required(
        "status",
        AttrType::Atom {
            one_of: &["OPEN", "IN_PROGRESS", "RESOLVED", "CLOSED"],
        },
    ),
    AttributeDef::required("priority", AttrType::Integer),
    AttributeDef::optional("author_id", AttrType::Uuid),
];

static TICKET_RELS: &[RelationshipDef] = &[
    RelationshipDef::belongs_to("author", || &REP_DEF, "author_id"),
];

static TICKET_VALIDATIONS: &[Validation] = &[
    Validation::present("title"),
    Validation::string_length("title", Some(5), Some(100)),
    Validation::one_of("status", &["OPEN", "IN_PROGRESS", "RESOLVED", "CLOSED"]),
    Validation::numericality("priority", Some(1), Some(5)),
];

static TICKET_ACTIONS: &[ActionDef] = &[
    ActionDef::read("read").primary(),
    ActionDef::create("open")
        .primary()
        .accept(&["title", "description", "status", "priority", "author_id"])
        .validations(TICKET_VALIDATIONS),
    ActionDef::update("change_status")
        .primary()
        .accept(&["status"]),
    ActionDef::update("update_details")
        .accept(&["title", "description", "priority"])
        .validations(TICKET_VALIDATIONS),
    ActionDef::destroy("close").primary(),
];

pub static TICKET_DEF: ResourceDef = ResourceDef {
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
    store_type_id: ash_core::default_store_type_id,
    store_name: "default",
};

pub static HELPDESK_DOMAIN: DomainDef = DomainDef {
    name: "Helpdesk",
    resources: &[&TICKET_DEF, &REP_DEF],
};

// --- TypeScript SDK Generator ---

pub fn emit_typescript_sdk(output_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let generator = TypeScriptGenerator::new()
        .config(
            TypeScriptConfig::new()
                .with_zod(true)
                .with_client(true)
                .with_react(true)
                .with_client_name("AshClient")
                .with_graphql_endpoint("/graphql"),
        )
        .add_domain(&HELPDESK_DOMAIN);

    let ts_code = generator.generate_consolidated()?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(output_path, ts_code)?;
    println!("  Generated TypeScript SDK -> {}", output_path.display());
    Ok(())
}

// --- CORS Middleware ---

pub async fn cors_middleware(req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS")
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type, Authorization")
            .body(axum::body::Body::empty())
            .unwrap();
    }

    let mut res = next.run(req).await;
    res.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    res.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS".parse().unwrap());
    res.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_HEADERS, "Content-Type, Authorization".parse().unwrap());
    res
}

async fn health_check() -> impl IntoResponse {
    "OK"
}

/// Creates a seeded Memory data layer and Ash GraphQL Axum Router.
pub async fn build_app() -> Result<Router, Box<dyn std::error::Error>> {
    let mem = Memory::new();
    let pubsub = PubSub::new();

    // 1. Seed Representatives
    let rep1_id = Uuid::new_v4();
    let mut r1 = FieldMap::new();
    r1.insert("id".into(), Value::Uuid(rep1_id));
    r1.insert("name".into(), Value::String("Sarah Chen".into()));
    r1.insert("email".into(), Value::String("sarah.chen@support.ash".into()));
    r1.insert("role".into(), Value::String("Support Lead".into()));
    mem.create(&REP_DEF, rep1_id, r1).await?;

    let rep2_id = Uuid::new_v4();
    let mut r2 = FieldMap::new();
    r2.insert("id".into(), Value::Uuid(rep2_id));
    r2.insert("name".into(), Value::String("David Kim".into()));
    r2.insert("email".into(), Value::String("david.kim@eng.ash".into()));
    r2.insert("role".into(), Value::String("Staff SRE".into()));
    mem.create(&REP_DEF, rep2_id, r2).await?;

    // 2. Seed Tickets
    let t1_id = Uuid::new_v4();
    let mut t1 = FieldMap::new();
    t1.insert("id".into(), Value::Uuid(t1_id));
    t1.insert("title".into(), Value::String("Postgres connection pool exhaustion during traffic spike".into()));
    t1.insert("description".into(), Value::String("API gateway latency spiked to 2.4s. Connection pool maxed at 50.".into()));
    t1.insert("status".into(), Value::String("IN_PROGRESS".into()));
    t1.insert("priority".into(), Value::Int(1));
    t1.insert("author_id".into(), Value::Uuid(rep2_id));
    mem.create(&TICKET_DEF, t1_id, t1).await?;

    let t2_id = Uuid::new_v4();
    let mut t2 = FieldMap::new();
    t2.insert("id".into(), Value::Uuid(t2_id));
    t2.insert("title".into(), Value::String("Password reset token expires prematurely on mobile Safari".into()));
    t2.insert("description".into(), Value::String("Multiple customer reports of 401 token expired within 2 minutes of requesting.".into()));
    t2.insert("status".into(), Value::String("OPEN".into()));
    t2.insert("priority".into(), Value::Int(2));
    t2.insert("author_id".into(), Value::Uuid(rep1_id));
    mem.create(&TICKET_DEF, t2_id, t2).await?;

    let t3_id = Uuid::new_v4();
    let mut t3 = FieldMap::new();
    t3.insert("id".into(), Value::Uuid(t3_id));
    t3.insert("title".into(), Value::String("Nightly customer SLA CSV report delivery timed out".into()));
    t3.insert("description".into(), Value::String("Cron job failed at 03:00 UTC. Email notification dispatched.".into()));
    t3.insert("status".into(), Value::String("RESOLVED".into()));
    t3.insert("priority".into(), Value::Int(4));
    t3.insert("author_id".into(), Value::Uuid(rep1_id));
    mem.create(&TICKET_DEF, t3_id, t3).await?;

    let ctx = Context::new(mem);

    let schema = AshGraphQL::from_resources(&[&TICKET_DEF, &REP_DEF])
        .with_pubsub(pubsub)
        .with_dataloader()
        .finish_with_context(ctx)?;

    let app = ash_graphql::axum::graphql_router(schema)
        .route("/health", get(health_check))
        .layer(middleware::from_fn(cors_middleware));

    Ok(app)
}
