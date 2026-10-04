//! The committed TypeScript SDK matches the resources and type-checks.

use std::path::Path;
use std::process::Command;

#[test]
fn committed_sdk_is_current() {
    let committed = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("sdk/lagrange.ts")).unwrap();
    let generated = lagrange::server::typescript_sdk(true).unwrap();
    assert!(
        committed == generated,
        "sdk/lagrange.ts is out of date; run `cargo run -p lagrange -- sdk` in examples/lagrange"
    );
}

#[test]
fn sdk_type_checks() {
    // Zod is a package import, so check the SDK without it; the types are the same.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lagrange.ts");
    std::fs::write(&file, lagrange::server::typescript_sdk(false).unwrap()).unwrap();
    let output = match Command::new("tsc")
        .args(["--noEmit", "--strict", "--target", "ES2022", "--moduleResolution", "node", "--skipLibCheck"])
        .arg(&file)
        .output()
    {
        Ok(output) => output,
        Err(_) => {
            assert!(std::env::var("CI").is_err(), "CI installs tsc");
            eprintln!("skipping: tsc is not installed");
            return;
        }
    };
    assert!(
        output.status.success(),
        "tsc rejected the SDK:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
