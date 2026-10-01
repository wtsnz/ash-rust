//! # Lagrange
//!
//! An interplanetary freight and docking platform. Shipping lines (the tenants) book
//! contracts, pack containers, and fly voyages between ports on planets and in orbit,
//! while the port authority runs the shared map of ports and berths.
//!
//! - [`world`]: planets, ports, berths and berth reservations, shared by every line.
//! - [`fleet`]: shipping lines, crew, ships and their transponders.
//! - [`cargo`]: contracts, containers, voyages and stowage.
//! - [`telemetry`]: transponder pings and the audit log, in their own database.
//! - [`ops`]: the workflows that tie them together, such as reserving a berth.

pub mod app;
pub mod cargo;
pub mod fleet;
pub mod ops;
pub mod seed;
pub mod server;
pub mod sim;
pub mod telemetry;
pub mod types;
pub mod world;

use ash_core::domain;
use std::path::PathBuf;

pub use app::Lagrange;
pub use cargo::{Container, Contract, Manifest, Stowage, Voyage};
pub use fleet::{CrewMember, Ship, ShippingLine, Transponder};
pub use telemetry::{OpsEvent, TelemetryPing, TelemetryStore};
pub use world::{Berth, BerthReservation, Planet, Port, Route};

domain! {
    World {
        resources {
            Planet;
            Port;
            Berth;
            BerthReservation;
        }
    }
}

domain! {
    Fleet {
        resources {
            ShippingLine;
            CrewMember;
            Transponder;
            Ship;
        }
    }
}

domain! {
    Cargo {
        resources {
            Contract;
            Container;
            Voyage;
            Stowage;
        }
    }
}

domain! {
    Telemetry {
        resources {
            TelemetryPing;
            OpsEvent;
        }
    }
}

/// The committed migrations for the fleet database, for SQLite and Postgres.
pub fn migrations_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations")
}
