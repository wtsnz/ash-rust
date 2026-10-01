//! Closed sets shared across the domains. Each is stored as its lower-case name and
//! checked by a generated `CHECK` constraint.

use ash_core::AshEnum;

/// What a body's surface is like to land on.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Atmosphere {
    Vacuum,
    Thin,
    Breathable,
    Toxic,
}

/// Where a port sits relative to its planet.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum PortKind {
    Ground,
    #[ash(string = "low_orbit")]
    LowOrbit,
    Geostationary,
    Lagrange,
}

/// The docking clamps a berth has. Heavy-lift clamps take any hull.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum ClampType {
    Standard,
    #[ash(string = "heavy_lift")]
    HeavyLift,
    Cryogenic,
}

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum ReservationStatus {
    Active,
    Released,
}

#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum ShipClass {
    Shuttle,
    Hauler,
    Tanker,
    Freighter,
}

/// Dangerous-goods classes that need a customs seal before launch.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum HazmatClass {
    Flammable,
    Corrosive,
    Radioactive,
    Cryogenic,
}

/// What a crew member may do. It reaches policies through the authenticated actor.
#[derive(AshEnum, Copy, Clone, Debug, PartialEq, Eq)]
pub enum Role {
    Dispatcher,
    Captain,
    Customs,
    Shipper,
    #[ash(string = "port_authority")]
    PortAuthority,
}
