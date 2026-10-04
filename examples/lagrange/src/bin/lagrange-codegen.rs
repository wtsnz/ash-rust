//! Writes migrations for the fleet database from the resources:
//!
//! ```bash
//! cargo run -p lagrange --bin lagrange-codegen -- add_cargo_holds --dialect postgres
//! cargo run -p lagrange --bin lagrange-codegen -- --check --dialect sqlite
//! ```
//!
//! Run it from `examples/lagrange`. The telemetry database is installed directly and
//! has no migrations.

fn main() -> std::process::ExitCode {
    cargo_ash::codegen::main(&[&lagrange::WORLD_DEF, &lagrange::FLEET_DEF, &lagrange::CARGO_DEF])
}
