//! Berth reservations: one ship per berth per window, across every line, even when
//! dispatchers race for it.

mod support;

use ash_core::{Context, Error, Resource, UtcDateTime};
use lagrange::ops;
use lagrange::types::ReservationStatus;
use lagrange::{Berth, BerthReservation};
use support::{Backend, FleetDb, Harness};

fn at(raw: &str) -> UtcDateTime {
    UtcDateTime::parse(raw).unwrap()
}

async fn windows_do_not_overlap<D: FleetDb>(h: Harness<D>) {
    let ceres = h.sol.port("PZZ").id;
    let (ada, mae) = (h.ctx("Ada"), h.ctx("Mae"));
    let long_haul = h.sol.ship("Long Haul").id;
    let dust_devil = h.sol.ship("Dust Devil").id;

    let held = ops::reserve_berth(ada, long_haul, ceres, "A1", at("2187-02-12T00:00:00Z"), at("2187-02-13T00:00:00Z"))
        .await
        .unwrap();

    // Another line can't take an overlapping window, though it can't see the hold.
    assert!(BerthReservation::query(mae).all().await.unwrap().is_empty());
    let err = ops::reserve_berth(mae, dust_devil, ceres, "A1", at("2187-02-12T12:00:00Z"), at("2187-02-14T00:00:00Z"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("already held"), "{err}");

    // Back to back is fine, and so is another berth.
    ops::reserve_berth(mae, dust_devil, ceres, "A1", at("2187-02-13T00:00:00Z"), at("2187-02-14T00:00:00Z"))
        .await
        .unwrap();
    ops::reserve_berth(mae, dust_devil, ceres, "A2", at("2187-02-12T12:00:00Z"), at("2187-02-13T12:00:00Z"))
        .await
        .unwrap();

    // Releasing a hold frees its window.
    held.release_on(ada).await.unwrap();
    let freed = ops::reserve_berth(mae, dust_devil, ceres, "A1", at("2187-02-12T06:00:00Z"), at("2187-02-12T18:00:00Z"))
        .await
        .unwrap();
    assert_eq!(freed.status, ReservationStatus::Active);

    // A line counts its own holds on a berth; the port authority counts every line's.
    let holds_on_a1 = |ctx: Context<D>| async move {
        Berth::query(&ctx)
            .filter(Berth::port_id.eq(ceres) & Berth::code.eq("A1"))
            .load_aggregate(Berth::active_reservations)
            .one()
            .await
            .unwrap()
            .active_reservations
    };
    assert_eq!(holds_on_a1(ada.clone()).await, Some(0));
    assert_eq!(holds_on_a1(mae.clone()).await, Some(2));
    assert_eq!(holds_on_a1(h.app.port_authority()).await, Some(2));

    // A window must end after it starts, and a 48,000 t tanker needs heavy-lift clamps.
    let err = ops::reserve_berth(ada, long_haul, ceres, "A2", at("2187-03-02T00:00:00Z"), at("2187-03-01T00:00:00Z"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("must end after it starts"), "{err}");
    let behemoth = h.sol.ship("Behemoth").id;
    let err = ops::reserve_berth(ada, behemoth, ceres, "A2", at("2187-03-01T00:00:00Z"), at("2187-03-02T00:00:00Z"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("too heavy"), "{err}");
    ops::reserve_berth(ada, behemoth, ceres, "H1", at("2187-03-01T00:00:00Z"), at("2187-03-02T00:00:00Z"))
        .await
        .unwrap();
}
on_every_backend!(windows_do_not_overlap);

/// Ten dispatchers ask for the same berth and window at once. Exactly one gets it.
async fn racing_dispatchers_get_one_berth<D: FleetDb>(h: Harness<D>) {
    let ceres = h.sol.port("PZZ").id;
    let attempts = (0..10).map(|n| {
        let (dispatcher, ship) = if n % 2 == 0 { ("Ada", "Long Haul") } else { ("Mae", "Dust Devil") };
        let ctx = h.ctx(dispatcher).clone();
        let ship = h.sol.ship(ship).id;
        tokio::spawn(async move {
            ops::reserve_berth(&ctx, ship, ceres, "C1", at("2187-04-01T00:00:00Z"), at("2187-04-02T00:00:00Z")).await
        })
    });
    let mut won = 0;
    for attempt in futures_util::future::join_all(attempts).await {
        if attempt.unwrap().is_ok() {
            won += 1;
        }
    }
    assert_eq!(won, 1, "exactly one dispatcher holds the berth");

    let anyone = h.app.port_authority();
    let held = BerthReservation::query(&anyone)
        .filter(BerthReservation::berth_code.eq("C1"))
        .count()
        .await
        .unwrap();
    assert_eq!(held, 1);
}
on_every_backend!(racing_dispatchers_get_one_berth);

/// On Postgres an exclusion constraint refuses overlapping holds even from code that
/// skips the workflow.
async fn postgres_backs_the_rule_up<D: FleetDb>(h: Harness<D>) {
    if h.backend != Backend::Postgres {
        return;
    }
    let ceres = h.sol.port("PZZ").id;
    let ada = h.ctx("Ada");
    let ship = h.sol.ship("Long Haul").id;
    ops::reserve_berth(ada, ship, ceres, "A1", at("2187-05-01T00:00:00Z"), at("2187-05-03T00:00:00Z"))
        .await
        .unwrap();
    let err = BerthReservation::reserve(ada)
        .port_id(ceres)
        .berth_code("A1")
        .ship_id(ship)
        .starts_at(at("2187-05-02T00:00:00Z"))
        .ends_at(at("2187-05-04T00:00:00Z"))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("no_overlap"), "{err}");
}
on_every_backend!(postgres_backs_the_rule_up);

/// Renaming a berth carries its reservations along through the composite foreign key.
async fn recoding_a_berth_moves_its_holds<D: FleetDb>(h: Harness<D>) {
    if h.backend == Backend::Memory {
        // ON UPDATE CASCADE is a database rule; the memory store has none.
        return;
    }
    let ceres = h.sol.port("PZZ").id;
    let hold = ops::reserve_berth(h.ctx("Ada"), h.sol.ship("Long Haul").id, ceres, "A2", at("2187-06-01T00:00:00Z"), at("2187-06-02T00:00:00Z"))
        .await
        .unwrap();
    let authority = h.app.port_authority();
    let berth = Berth::get_by_port_code(&authority, ceres, "A2".to_string()).await.unwrap();
    berth.recode_on(&authority).code("A2-WEST").await.unwrap();

    let moved = BerthReservation::get(h.ctx("Ada"), hold.id).await.unwrap();
    assert_eq!(moved.berth_code, "A2-WEST");

    // A berth with holds can't be torn down.
    let err = h.app.fleet().destroy(&Berth::DEF, None, berth.id.into()).await.unwrap_err();
    assert!(!matches!(err, Error::NotFound), "{err:?}");
}
on_every_backend!(recoding_a_berth_moves_its_holds);
