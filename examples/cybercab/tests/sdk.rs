//! The frontend's generated SDK matches the domain, and the frontend type-checks against it.

use std::path::Path;
use std::process::Command;

#[test]
fn committed_sdk_is_current() {
    let committed = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend/src/lib/ash.ts"),
    )
    .unwrap();
    let generated = cybercab::server::typescript_sdk().unwrap();
    assert!(
        committed == generated,
        "frontend/src/lib/ash.ts is out of date; run `cargo run -p cybercab -- --codegen-only`"
    );
}

#[test]
fn frontend_type_checks() {
    let frontend = Path::new(env!("CARGO_MANIFEST_DIR")).join("frontend");
    if !frontend.join("node_modules").exists() {
        assert!(
            std::env::var("CI").is_err(),
            "CI installs the frontend's dependencies"
        );
        eprintln!("skipping: run `bun install` in examples/cybercab/frontend");
        return;
    }
    let output = Command::new("node")
        .args(["node_modules/typescript/bin/tsc", "--noEmit", "-p", "."])
        .current_dir(&frontend)
        .output()
        .expect("node runs tsc");
    assert!(
        output.status.success(),
        "tsc rejected the frontend:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
