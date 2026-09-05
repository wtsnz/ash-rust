use std::fs;
use std::path::PathBuf;
use ash_sql::TableSnapshot;
use ash_typescript::{generate_from_snapshots, TypeScriptConfig};
use clap::Args;

#[derive(Args, Debug)]
pub struct TypeScriptArgs {
    /// Directory containing database snapshot JSON files
    #[arg(long, default_value = "snapshots")]
    pub snapshots: PathBuf,

    /// Output file path for generated TypeScript code (e.g. ./frontend/src/ash.ts)
    #[arg(long, short = 'o', default_value = "src/ash.ts")]
    pub out: PathBuf,

    /// Disable Zod validation schema generation
    #[arg(long)]
    pub no_zod: bool,

    /// Disable client SDK runtime generation
    #[arg(long)]
    pub no_client: bool,

    /// Disable React / TanStack Query hook helpers
    #[arg(long)]
    pub no_react: bool,

    /// Name of the root client class
    #[arg(long, default_value = "AshClient")]
    pub client_name: String,

    /// Relative or absolute GraphQL endpoint URL
    #[arg(long, default_value = "/graphql")]
    pub endpoint: String,
}

pub fn run(args: TypeScriptArgs) -> Result<(), Box<dyn std::error::Error>> {
    if !args.snapshots.exists() {
        return Err(format!(
            "Snapshots directory `{}` does not exist. Run `cargo ash schema dump` first.",
            args.snapshots.display()
        )
        .into());
    }

    let mut snapshots = Vec::new();
    let entries = fs::read_dir(&args.snapshots)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("json") {
            let content = fs::read_to_string(&path)?;
            let snapshot: TableSnapshot = serde_json::from_str(&content)?;
            snapshots.push(snapshot);
        }
    }

    if snapshots.is_empty() {
        return Err(format!(
            "No snapshot JSON files found in `{}`. Run `cargo ash schema dump` first.",
            args.snapshots.display()
        )
        .into());
    }

    // Sort snapshots deterministically by table name
    snapshots.sort_by(|a, b| a.table.cmp(&b.table));

    let config = TypeScriptConfig::new()
        .with_zod(!args.no_zod)
        .with_client(!args.no_client)
        .with_react(!args.no_react)
        .with_client_name(&args.client_name)
        .with_graphql_endpoint(&args.endpoint);

    let ts_code = generate_from_snapshots(&snapshots, &config)?;

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    fs::write(&args.out, ts_code)?;

    println!(
        "Successfully generated TypeScript SDK ({} tables) to `{}`.",
        snapshots.len(),
        args.out.display()
    );

    Ok(())
}
