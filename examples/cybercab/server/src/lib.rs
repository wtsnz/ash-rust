//! # Cybercab Command Center
//!
//! The software a robotaxi operations floor runs on: every Cybercab in Austin on one map,
//! live, with the rides they're giving, the riders aboard, and the alerts that need a
//! person. The domain is split into four bounded contexts, each an Ash domain:
//!
//! - [`fleet`]: the cabs and the Supercharger hubs they return to.
//! - [`riders`]: the people who hail them.
//! - [`rides`]: trips, the core of the business, and the zones demand is tracked in.
//! - [`telemetry`]: what the fleet reports and what the control room watches.
//!
//! [`sim`] stands in for the real fleet: it drives cabs along real Austin streets and
//! talks to the domain only through its actions, exactly as an operator's console does.

pub mod city;
pub mod fleet;
pub mod riders;
pub mod rides;
pub mod rng;
pub mod seed;
pub mod server;
pub mod sim;
pub mod telemetry;
pub mod types;

use ash_core::domain;

pub use fleet::{Cab, Depot};
pub use riders::Rider;
pub use rides::{ServiceZone, Trip};
pub use telemetry::{FleetAlert, PulseSample, TelemetrySample};

domain! {
    Fleet {
        resources {
            Cab;
            Depot;
        }
    }
}

domain! {
    Riders {
        resources {
            Rider;
        }
    }
}

domain! {
    Rides {
        resources {
            Trip;
            ServiceZone;
        }
    }
}

domain! {
    Telemetry {
        resources {
            TelemetrySample;
            FleetAlert;
            PulseSample;
        }
    }
}

/// Every domain, for the API and the TypeScript SDK.
pub const DOMAINS: [&ash_core::DomainDef; 4] =
    [&FLEET_DEF, &RIDERS_DEF, &RIDES_DEF, &TELEMETRY_DEF];
