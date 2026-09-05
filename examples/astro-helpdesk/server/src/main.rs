use std::path::Path;
use astro_helpdesk::{build_app, emit_typescript_sdk};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let sdk_path = Path::new("frontend/src/lib/ash.ts");

    // Always ensure TypeScript SDK is emitted
    emit_typescript_sdk(sdk_path)?;

    if args.iter().any(|a| a == "--codegen-only") {
        println!("Codegen completed. Exiting.");
        return Ok(());
    }

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(4000);

    let app = build_app().await?;
    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;

    println!("\n========================================================");
    println!("  🦀 ash-rust Helpdesk GraphQL Server is running!");
    println!("========================================================");
    println!("  GraphQL API:       http://{}/graphql", addr);
    println!("  GraphiQL IDE:      http://{}/graphiql", addr);
    println!("  Health Check:      http://{}/health", addr);
    println!("========================================================\n");

    axum::serve(listener, app).await?;
    Ok(())
}
