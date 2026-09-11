pub mod representative;
pub mod ticket;

pub use representative::{REPRESENTATIVE_DEF, Representative};
pub use ticket::{TICKET_DEF, Ticket, TicketActions, TicketStatus};

use ash_core::{Context, domain};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use ash_pubsub::PubSub;
use ash_typescript::{TypeScriptConfig, TypeScriptGenerator};
use axum::Router;
use axum::extract::Request;
use axum::http::{Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use std::path::Path;

// --- Domain Definition via domain! DSL ---

domain! {
    domain Helpdesk;
    resources {
        Ticket,
        Representative,
    }
}

pub const HELPDESK_DOMAIN: ash_core::DomainDef = HELPDESK_DEF;

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
        .add_domain(&HELPDESK_DEF);

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
            .header(
                header::ACCESS_CONTROL_ALLOW_HEADERS,
                "Content-Type, Authorization",
            )
            .body(axum::body::Body::empty())
            .unwrap();
    }

    let mut res = next.run(req).await;
    res.headers_mut()
        .insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".parse().unwrap());
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        "GET, POST, OPTIONS".parse().unwrap(),
    );
    res.headers_mut().insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        "Content-Type, Authorization".parse().unwrap(),
    );
    res
}

async fn health_check() -> impl IntoResponse {
    "OK"
}

/// Creates a seeded Memory data layer and Ash GraphQL Axum Router
/// using the Ash Resource and Context DSL.
pub async fn build_app() -> Result<Router, Box<dyn std::error::Error>> {
    let mem = Memory::new();
    let pubsub = PubSub::new();
    let ctx = Context::new(mem);

    // 1. Seed Representatives using Resource / Context DSL
    let rep1 = Representative::create(&ctx)
        .name("Sarah Chen")
        .email("sarah.chen@support.ash")
        .role("Support Lead")
        .await?;

    let rep2 = Representative::create(&ctx)
        .name("David Kim")
        .email("david.kim@eng.ash")
        .role("Staff SRE")
        .await?;

    // 2. Seed Tickets using Resource / Context DSL
    Ticket::open(&ctx)
        .title("Postgres connection pool exhaustion during traffic spike")
        .description("API gateway latency spiked to 2.4s. Connection pool maxed at 50.")
        .status(crate::ticket::TicketStatus::InProgress)
        .priority(1i64)
        .author_id(rep2.id)
        .await?;

    Ticket::open(&ctx)
        .title("Password reset token expires prematurely on mobile Safari")
        .description(
            "Multiple customer reports of 401 token expired within 2 minutes of requesting.",
        )
        .status(crate::ticket::TicketStatus::Open)
        .priority(2i64)
        .author_id(rep1.id)
        .await?;

    Ticket::open(&ctx)
        .title("Nightly customer SLA CSV report delivery timed out")
        .description("Cron job failed at 03:00 UTC. Email notification dispatched.")
        .status(crate::ticket::TicketStatus::Resolved)
        .priority(4i64)
        .author_id(rep1.id)
        .await?;

    let schema = AshGraphQL::from_domain(&HELPDESK_DEF)
        .with_pubsub(pubsub)
        .with_dataloader()
        .finish_with_context(ctx)?;

    let app = ash_graphql::axum::graphql_router(schema)
        .route("/health", get(health_check))
        .layer(middleware::from_fn(cors_middleware));

    Ok(app)
}
