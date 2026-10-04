//! A voyage from booking to delivery, and the rules that hold along the way.

mod support;

use ash_core::{CiString, Date, Decimal, Error, ResourceExt, UtcDateTime};
use lagrange::ops::{self, Slot};
use lagrange::seed::HELIOS;
use lagrange::types::HazmatClass;
use lagrange::{Container, Contract, Voyage};
use support::{FleetDb, Harness};

fn at(raw: &str) -> UtcDateTime {
    UtcDateTime::parse(raw).unwrap()
}

fn day(raw: &str) -> Date {
    Date::parse(raw).unwrap()
}

/// Books a contract from LEO to Ceres and packs `masses` containers into it.
async fn booked_cargo<D: FleetDb>(h: &Harness<D>, reference: &str, masses: &[i64]) -> (Contract, Vec<Container>) {
    let grace = h.ctx("Grace");
    let contract = Contract::book(grace)
        .external_ref(reference)
        .shipper_email(CiString::parse("grace@helios-freight.example").unwrap())
        .origin_port_id(h.sol.port("LEO").id)
        .destination_port_id(h.sol.port("PZZ").id)
        .freight_credits(12_000)
        .insured_value(Decimal::parse("250000.00").unwrap())
        .deliver_by(day("2187-03-01"))
        .await
        .unwrap();
    let mut containers = Vec::new();
    for (n, mass) in masses.iter().enumerate() {
        containers.push(
            Container::pack(grace)
                .code(CiString::parse(&format!("{reference}-{n}")).unwrap())
                .contract_id(contract.id)
                .mass_tonnes(*mass)
                .await
                .unwrap(),
        );
    }
    (contract, containers)
}

async fn full_voyage<D: FleetDb>(h: Harness<D>) {
    let (contract, containers) = booked_cargo(&h, "GR-0001", &[22, 18]).await;
    let ship = h.sol.ship("Long Haul");
    let ada = h.ctx("Ada");

    let voyage = ops::plan_voyage(ada, ship, h.sol.port("LEO").id, h.sol.port("PZZ").id, day("2187-02-01"))
        .await
        .unwrap();
    assert_eq!(voyage.current_state(), "planned");
    assert!(voyage.transit_hours > 6, "{} h", voyage.transit_hours);

    for (n, container) in containers.iter().enumerate() {
        ops::stow(ada, voyage.id, container.id, Slot { bay: 1, stack: 1, tier: n as i64 + 1 })
            .await
            .unwrap();
    }
    let loaded = Voyage::query(ada)
        .load_aggregate(Voyage::container_count)
        .load_aggregate(Voyage::loaded_mass)
        .load_aggregate(Voyage::carries_hazmat)
        .one()
        .await
        .unwrap();
    assert_eq!(loaded.container_count, Some(2));
    assert_eq!(loaded.loaded_mass, Some(40));
    assert_eq!(loaded.carries_hazmat, Some(false));
    assert_eq!(contract.reload(ada).await.unwrap().current_state(), "loaded");

    // The captain can't launch before customs clears the voyage...
    let yuri = h.ctx("Yuri");
    let err = ops::launch(yuri, voyage.id, at("2187-02-01T06:00:00Z")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden | Error::Extension(_)), "{err:?}");
    ops::clear_customs(&h.customs(HELIOS), voyage.id).await.unwrap();
    // ...or without a berth held at the destination.
    let err = ops::launch(yuri, voyage.id, at("2187-02-01T06:00:00Z")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");

    let reservation = ops::reserve_berth(ada, ship.id, h.sol.port("PZZ").id, "A1", at("2187-02-12T00:00:00Z"), at("2187-02-13T00:00:00Z"))
        .await
        .unwrap();
    let voyage = Voyage::get(ada, voyage.id).await.unwrap();
    voyage.hold_berth_on(ada).reservation_id(reservation.id).await.unwrap();

    // Only the voyage's own captain may fly it.
    let err = ops::launch(h.ctx("Valentina"), voyage.id, at("2187-02-01T06:00:00Z")).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");

    ops::launch(yuri, voyage.id, at("2187-02-01T06:00:00Z")).await.unwrap();
    assert_eq!(ship.reload(ada).await.unwrap().current_state(), "in_transit");
    assert_eq!(contract.reload(ada).await.unwrap().current_state(), "in_transit");

    ops::arrive(yuri, voyage.id, at("2187-02-12T03:00:00Z")).await.unwrap();
    assert_eq!(contract.reload(ada).await.unwrap().current_state(), "delivered");
    let notice = h.mailer.last_delivered().expect("a delivery notice");
    assert_eq!(notice.subject.as_deref(), Some("Delivered: GR-0001"));

    let done = ops::complete(ada, voyage.id).await.unwrap();
    assert_eq!(done.current_state(), "completed");
    assert_eq!(contract.reload(ada).await.unwrap().current_state(), "closed");
    let released = lagrange::BerthReservation::get(ada, reservation.id).await.unwrap();
    assert_eq!(released.status, lagrange::types::ReservationStatus::Released);
}
on_every_backend!(full_voyage);

async fn dangerous_goods_need_a_seal<D: FleetDb>(h: Harness<D>) {
    let grace = h.ctx("Grace");
    let (contract, _) = booked_cargo(&h, "GR-0002", &[]).await;
    let fuel = Container::pack(grace)
        .code(CiString::parse("HAZ-1").unwrap())
        .contract_id(contract.id)
        .mass_tonnes(30)
        .hazmat(HazmatClass::Flammable)
        .await
        .unwrap();
    assert!(fuel.hazardous);

    let ada = h.ctx("Ada");
    let voyage = ops::plan_voyage(ada, h.sol.ship("Long Haul"), h.sol.port("LEO").id, h.sol.port("PZZ").id, day("2187-02-01"))
        .await
        .unwrap();
    ops::stow(ada, voyage.id, fuel.id, Slot { bay: 2, stack: 1, tier: 1 }).await.unwrap();
    let carries = Voyage::query(ada).load_aggregate(Voyage::carries_hazmat).one().await.unwrap();
    assert_eq!(carries.carries_hazmat, Some(true));

    let customs = h.customs(HELIOS);
    let err = ops::clear_customs(&customs, voyage.id).await.unwrap_err();
    assert!(err.to_string().contains("HAZ-1"), "{err}");

    // Only customs seals, and the seal itself is hidden from the line's dispatcher.
    let seal = ash_core::Binary::from_bytes(*b"sealed by customs 2187-01-30");
    let err = fuel.clone().seal_on(ada).seal(seal.clone()).await.unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    let sealed = Container::get(&customs, fuel.id).await.unwrap().seal_on(&customs).seal(seal.clone()).await.unwrap();
    assert!(sealed.sealed);
    assert_eq!(Container::get(&customs, fuel.id).await.unwrap().seal, Some(seal));
    assert_eq!(Container::get(ada, fuel.id).await.unwrap().seal, None);

    ops::clear_customs(&customs, voyage.id).await.unwrap();
}
on_every_backend!(dangerous_goods_need_a_seal);

async fn holds_have_limits<D: FleetDb>(h: Harness<D>) {
    let ada = h.ctx("Ada");
    // Tin Kettle carries three containers.
    let (_, containers) = booked_cargo(&h, "GR-0003", &[5, 5, 5, 5]).await;
    let voyage = ops::plan_voyage(ada, h.sol.ship("Tin Kettle"), h.sol.port("LEO").id, h.sol.port("PZZ").id, day("2187-02-01"))
        .await
        .unwrap();
    for (n, container) in containers.iter().take(3).enumerate() {
        ops::stow(ada, voyage.id, container.id, Slot { bay: 1, stack: 1, tier: n as i64 + 1 }).await.unwrap();
    }
    let err = ops::stow(ada, voyage.id, containers[3].id, Slot { bay: 1, stack: 2, tier: 1 }).await.unwrap_err();
    assert!(err.to_string().contains("no free slots"), "{err}");

    // A slot holds one container, and the hold has walls.
    let (_, more) = booked_cargo(&h, "GR-0004", &[5]).await;
    let roomy = ops::plan_voyage(ada, h.sol.ship("Long Haul"), h.sol.port("LEO").id, h.sol.port("PZZ").id, day("2187-02-02"))
        .await
        .unwrap();
    ops::stow(ada, roomy.id, containers[3].id, Slot { bay: 3, stack: 3, tier: 3 }).await.unwrap();
    let err = ops::stow(ada, roomy.id, more[0].id, Slot { bay: 3, stack: 3, tier: 3 }).await.unwrap_err();
    assert!(matches!(err, Error::IdentityConflict { .. }), "{err:?}");
    if h.backend != support::Backend::Memory {
        let err = ops::stow(ada, roomy.id, more[0].id, Slot { bay: 99, stack: 1, tier: 1 }).await.unwrap_err();
        assert!(err.to_string().to_lowercase().contains("check"), "{err}");
    }

    // A ship without a captain can't be planned.
    let err = ops::plan_voyage(ada, h.sol.ship("Behemoth"), h.sol.port("LEO").id, h.sol.port("PZZ").id, day("2187-02-01"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no captain"), "{err}");
}
on_every_backend!(holds_have_limits);
