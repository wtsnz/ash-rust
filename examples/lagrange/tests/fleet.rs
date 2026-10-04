//! Crew, tenancy, and ships: who can sign in, what each line can see, and retiring a
//! ship.

mod support;

use ash_core::{CiString, Error, ResourceExt, UtcDateTime};
use lagrange::seed::{HELIOS, PASSWORD, RED_DUST};
use lagrange::types::{Role, ShipClass};
use lagrange::{BerthReservation, CrewMember, Port, Ship, ops};
use support::{Backend, FleetDb, Harness};

async fn crew_sign_in<D: FleetDb>(h: Harness<D>) {
    // Emails are case-insensitive, and the session carries the member's line.
    let ada = h.app.sign_in("ADA@helios-freight.example", PASSWORD).await.unwrap();
    assert_eq!(ada.tenant(), Some(HELIOS));
    assert_eq!(ada.actor.as_ref().and_then(|a| a.role()), Some("dispatcher"));
    assert!(h.app.sign_in("ada@helios-freight.example", "wrong-password!").await.is_err());
    assert!(h.app.sign_in("nobody@helios-freight.example", PASSWORD).await.is_err());

    // Dispatchers hire crew into their own line; shippers can't.
    let hired = CrewMember::register_with_password(&ada)
        .email(CiString::parse("ivy@helios-freight.example").unwrap())
        .name("Ivy")
        .role(Role::Captain)
        .password("a-long-enough-password")
        .await
        .unwrap();
    assert_eq!(hired.line, HELIOS);
    assert!(hired.hashed_password.as_deref().unwrap().starts_with("$argon2"));
    let err = CrewMember::register_with_password(h.ctx("Grace"))
        .email(CiString::parse("eve@helios-freight.example").unwrap())
        .name("Eve")
        .role(Role::Dispatcher)
        .password("a-long-enough-password")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    let err = CrewMember::register_with_password(&ada)
        .email(CiString::parse("short@helios-freight.example").unwrap())
        .name("Short")
        .role(Role::Captain)
        .password("tiny")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("10"), "{err}");
}
on_every_backend!(crew_sign_in);

async fn lines_see_their_own_fleet<D: FleetDb>(h: Harness<D>) {
    let (ada, mae) = (h.ctx("Ada"), h.ctx("Mae"));
    let names = |ships: Vec<Ship>| {
        let mut names: Vec<String> = ships.into_iter().map(|s| s.name).collect();
        names.sort();
        names
    };
    assert_eq!(names(Ship::query(ada).all().await.unwrap()), ["Behemoth", "Long Haul", "Tin Kettle"]);
    assert_eq!(names(Ship::query(mae).all().await.unwrap()), ["Dust Devil"]);
    let long_haul = h.sol.ship("Long Haul");
    assert!(matches!(Ship::get(mae, long_haul.id).await, Err(Error::NotFound)));

    // The map is shared, and anyone may read it. Read policies filter rather than
    // fail, so a reader who isn't signed in sees no ships at all.
    assert_eq!(Port::query(&h.app.anonymous()).count().await.unwrap(), 7);
    assert_eq!(Port::query(mae).count().await.unwrap(), 7);
    let stranger = h.app.anonymous().with_tenant(HELIOS);
    assert!(Ship::query(&stranger).all().await.unwrap().is_empty());
    assert!(matches!(Ship::get(&stranger, long_haul.id).await, Err(Error::NotFound)));

    // Ships are created in the dispatcher's line, never another.
    let commissioned = Ship::commission(mae)
        .registry(CiString::parse("RD-102").unwrap())
        .name("Sandstorm")
        .class(ShipClass::Hauler)
        .dry_mass_tonnes(2_000)
        .slot_capacity(2)
        .await
        .unwrap();
    assert_eq!(commissioned.line, RED_DUST);
}
on_every_backend!(lines_see_their_own_fleet);

async fn captains_fly_their_own_ships<D: FleetDb>(h: Harness<D>) {
    let long_haul = h.sol.ship("Long Haul").clone();
    let err = long_haul.clone().depart_on(h.ctx("Valentina")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    let flying = long_haul.depart_on(h.ctx("Yuri")).await.unwrap();
    assert_eq!(flying.current_state(), "in_transit");
    assert_eq!(flying.version, 2);

    // Maintenance is the dispatcher's call, and only for a docked ship.
    let kettle = h.sol.ship("Tin Kettle").clone();
    let err = kettle.clone().begin_maintenance_on(h.ctx("Valentina")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    let in_dock = kettle.begin_maintenance_on(h.ctx("Ada")).await.unwrap();
    assert_eq!(in_dock.current_state(), "maintenance");
    assert!(in_dock.clone().depart_on(h.ctx("Ada")).await.is_err());
    assert_eq!(in_dock.end_maintenance_on(h.ctx("Ada")).await.unwrap().current_state(), "docked");

    // A stale copy can't overwrite a newer one: departing raised the lock version.
    let stale = h.sol.ship("Long Haul").clone();
    let err = stale
        .assign_captain_on(h.ctx("Ada"))
        .captain_id(h.sol.crew("Valentina").id)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::StaleRecord { .. }), "{err:?}");
}
on_every_backend!(captains_fly_their_own_ships);

async fn decommissioned_ships_are_archived<D: FleetDb>(h: Harness<D>) {
    let ada = h.ctx("Ada");
    let kettle = h.sol.ship("Tin Kettle");
    let hold = ops::reserve_berth(
        ada,
        kettle.id,
        h.sol.port("SHK").id,
        "A1",
        UtcDateTime::parse("2187-08-01T00:00:00Z").unwrap(),
        UtcDateTime::parse("2187-08-02T00:00:00Z").unwrap(),
    )
    .await
    .unwrap();

    ops::decommission(ada, kettle.id).await.unwrap();
    assert!(matches!(Ship::get(ada, kettle.id).await, Err(Error::NotFound)));
    let retired = Ship::decommissioned(ada).all().await.unwrap();
    assert_eq!(retired.len(), 1);
    assert!(retired[0].is_archived());
    // Its berth hold went with it.
    assert!(matches!(BerthReservation::get(ada, hold.id).await, Err(Error::NotFound)));

    // The registry code is free for a new hull while the old one stays on file.
    let reborn = Ship::commission(ada)
        .registry(CiString::parse("hf-002").unwrap())
        .name("Tin Kettle II")
        .class(ShipClass::Hauler)
        .dry_mass_tonnes(2_500)
        .slot_capacity(3)
        .await
        .unwrap();
    if h.backend != Backend::Memory {
        // The partial unique index still guards live hulls.
        let err = Ship::commission(ada)
            .registry(CiString::parse("HF-002").unwrap())
            .name("Impostor")
            .class(ShipClass::Hauler)
            .dry_mass_tonnes(2_500)
            .slot_capacity(3)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::IdentityConflict { .. }), "{err:?}");
    }
    assert_eq!(reborn.reload(ada).await.unwrap().name, "Tin Kettle II");
}
on_every_backend!(decommissioned_ships_are_archived);
