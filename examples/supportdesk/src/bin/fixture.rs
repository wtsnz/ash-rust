//! Writes the fixture both supportdesk apps load: `fixture --out fixture.json [--seed N]`.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let out = flag("--out").unwrap_or_else(|| "fixture.json".into());
    let seed = flag("--seed").and_then(|s| s.parse().ok()).unwrap_or(0x5D_E5C);
    let fixture = supportdesk::fixture::generate(seed);
    std::fs::write(&out, serde_json::to_vec(&fixture)?)?;
    let count = |key: &str| fixture[key].as_array().map_or(0, Vec::len);
    println!(
        "{out}: {} orgs, {} agents, {} tags, {} tickets, {} comments, {} ticket tags",
        count("orgs"), count("agents"), count("tags"), count("tickets"), count("comments"), count("ticket_tags")
    );
    Ok(())
}
