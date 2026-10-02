//! A small Sol system to fly in: five bodies, their ports and berths, and two shipping
//! lines with crew, ships and transponders.

use std::collections::BTreeMap;

use ash_authentication::ApiKeyService;
use ash_core::{CiString, Context, DataLayer, Float, Inet, Result, Vector};
use uuid::Uuid;

use crate::types::{Atmosphere, ClampType, PortKind, Role, ShipClass};
use crate::{Berth, CrewMember, Lagrange, Planet, Port, Ship, ShippingLine, Transponder};

pub const HELIOS: &str = "helios-freight";
pub const RED_DUST: &str = "red-dust-logistics";
pub const CUSTOMS: &str = "sol-customs";

/// The password every seeded account signs in with.
pub const PASSWORD: &str = "correct-horse-battery";

/// What the seed created, by the names the tests and the simulation use.
pub struct Sol {
    pub planets: BTreeMap<&'static str, Planet>,
    /// Ports by code, such as `"LEO"` or `"PZZ"`.
    pub ports: BTreeMap<&'static str, Port>,
    /// Crew by first name.
    pub crew: BTreeMap<&'static str, CrewMember>,
    /// Ships by name.
    pub ships: BTreeMap<&'static str, Ship>,
    /// Each ship's transponder API key, by ship name.
    pub api_keys: BTreeMap<&'static str, String>,
}

impl Sol {
    pub fn port(&self, code: &str) -> &Port {
        &self.ports[code]
    }

    pub fn crew(&self, name: &str) -> &CrewMember {
        &self.crew[name]
    }

    pub fn ship(&self, name: &str) -> &Ship {
        &self.ships[name]
    }
}

fn vector(x: f32, y: f32, z: f32) -> Vector<3> {
    Vector::new([x, y, z]).expect("three finite coordinates")
}

fn float(value: &str) -> Float {
    Float::parse(value).expect("a finite number")
}

fn inet(value: &str) -> Inet {
    Inet::parse(value).expect("an IP address")
}

fn ci(value: &str) -> CiString {
    CiString::parse(value).expect("a string")
}

/// Charts the system and staffs two lines. Run once on an empty fleet database.
pub async fn sol<D>(app: &Lagrange<D>) -> Result<Sol>
where
    D: DataLayer + Clone + Send + Sync + 'static,
{
    let authority = app.port_authority();

    let mut planets = BTreeMap::new();
    for (name, gravity, position, atmosphere) in [
        ("Earth", "9.81", vector(1.0, 0.0, 0.0), Atmosphere::Breathable),
        ("Luna", "1.62", vector(1.0026, 0.0, 0.0), Atmosphere::Vacuum),
        ("Mars", "3.72", vector(-1.52, 0.1, 0.03), Atmosphere::Thin),
        ("Ceres", "0.28", vector(0.5, 2.7, 0.2), Atmosphere::Vacuum),
        ("Europa", "1.31", vector(-4.9, 1.6, 0.1), Atmosphere::Vacuum),
    ] {
        let planet = Planet::chart(&authority)
            .name(ci(name))
            .surface_gravity(float(gravity))
            .position(position)
            .atmosphere(atmosphere)
            .await?;
        planets.insert(name, planet);
    }

    let mut ports = BTreeMap::new();
    for (planet, code, name, kind, relay) in [
        ("Earth", "KSC", "Kennedy Ground", PortKind::Ground, "10.0.1.1"),
        ("Earth", "LEO", "Gateway Station", PortKind::LowOrbit, "10.0.2.1"),
        ("Luna", "SHK", "Shackleton Base", PortKind::Ground, "10.1.0.1"),
        ("Mars", "OLY", "Olympus Field", PortKind::Ground, "10.4.0.1"),
        ("Mars", "DMS", "Deimos Anchorage", PortKind::Lagrange, "10.4.1.1"),
        ("Ceres", "PZZ", "Piazzi Station", PortKind::Geostationary, "10.7.0.1"),
        ("Europa", "CNM", "Conamara Ice Port", PortKind::Ground, "fd00:5::1"),
    ] {
        let port = Port::open(&authority)
            .planet_id(planets[planet].id)
            .code(ci(code))
            .name(name)
            .kind(kind)
            .relay(inet(relay))
            .await?;
        for (berth, clamp, max_mass) in [
            ("A1", ClampType::Standard, 8_000),
            ("A2", ClampType::Standard, 8_000),
            ("H1", ClampType::HeavyLift, 60_000),
            ("C1", ClampType::Cryogenic, 12_000),
        ] {
            Berth::build(&authority)
                .port_id(port.id)
                .code(berth)
                .clamp(clamp)
                .max_mass_tonnes(max_mass)
                .await?;
        }
        ports.insert(code, port);
    }

    for (slug, name) in [
        (HELIOS, "Helios Freight"),
        (RED_DUST, "Red Dust Logistics"),
        (CUSTOMS, "Sol Customs Service"),
    ] {
        ShippingLine::register(&authority)
            .slug(ci(slug))
            .name(name)
            .dispatch_email(ci(&format!("dispatch@{slug}.example")))
            .await?;
    }

    let mut crew = BTreeMap::new();
    for (line, first, role) in [
        (HELIOS, "Ada", Role::Dispatcher),
        (HELIOS, "Yuri", Role::Captain),
        (HELIOS, "Valentina", Role::Captain),
        (HELIOS, "Grace", Role::Shipper),
        (RED_DUST, "Mae", Role::Dispatcher),
        (RED_DUST, "Neil", Role::Captain),
        (RED_DUST, "Sally", Role::Shipper),
        (CUSTOMS, "Chen", Role::Customs),
    ] {
        let member = CrewMember::register_with_password(&authority.clone().with_tenant(line))
            .email(ci(&format!("{}@{line}.example", first.to_lowercase())))
            .name(first)
            .role(role)
            .password(PASSWORD)
            .await?;
        crew.insert(first, member);
    }

    let keys = ApiKeyService::new("lgx_");
    let mut ships = BTreeMap::new();
    let mut api_keys = BTreeMap::new();
    for (dispatcher, name, registry, class, mass, slots, captain) in [
        ("Ada", "Long Haul", "HF-001", ShipClass::Freighter, 7_500, 6, Some("Yuri")),
        ("Ada", "Tin Kettle", "HF-002", ShipClass::Hauler, 2_400, 3, Some("Valentina")),
        ("Ada", "Behemoth", "HF-003", ShipClass::Tanker, 48_000, 12, None),
        ("Mae", "Dust Devil", "RD-101", ShipClass::Freighter, 7_000, 6, Some("Neil")),
    ] {
        let dispatch = app.as_crew(&crew[dispatcher]);
        let ship = Ship::commission(&dispatch)
            .registry(ci(registry))
            .name(name)
            .class(class)
            .dry_mass_tonnes(mass)
            .slot_capacity(slots)
            .captain_id(captain.map(|captain| crew[captain].id))
            .await?;
        let (raw_key, key_hash) = keys.generate_api_key();
        Transponder::install(&dispatch)
            .ship_id(ship.id)
            .label(format!("{name} transponder"))
            .api_key_hash(key_hash)
            .await?;
        api_keys.insert(name, raw_key);
        ships.insert(name, ship);
    }

    Ok(Sol {
        planets,
        ports,
        crew,
        ships,
        api_keys,
    })
}

/// Contexts for every seeded crew member, signed in.
pub fn contexts<D>(app: &Lagrange<D>, sol: &Sol) -> BTreeMap<&'static str, Context<D>>
where
    D: DataLayer + Clone + Send + Sync + 'static,
{
    sol.crew
        .iter()
        .map(|(name, member)| (*name, app.as_crew(member)))
        .collect()
}

/// An id that exists nowhere, for tests of missing records.
pub fn nowhere() -> Uuid {
    Uuid::from_u128(0xdead_beef)
}
