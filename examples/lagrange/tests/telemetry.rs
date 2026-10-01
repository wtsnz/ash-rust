//! Transponders report positions with an API key, and every fleet action is audited.
//! Both land in the telemetry database, not the fleet database.

mod support;

use ash_core::{CiString, Date, Decimal, Error, Float, Inet, UtcDateTime, Vector};
use lagrange::{Contract, OpsEvent, TelemetryPing};
use support::{FleetDb, Harness};

async fn telemetry_rows(h: &Harness<impl FleetDb>, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(h.app.telemetry().pool().unwrap())
        .await
        .unwrap()
}

async fn transponders_report_positions<D: FleetDb>(h: Harness<D>) {
    let ship = h.sol.ship("Long Haul");
    let transponder = h.app.transponder(&h.sol.api_keys["Long Haul"]).await.unwrap();
    for minute in 0..25 {
        TelemetryPing::report(&transponder)
            .position(Vector::new([1.0 - minute as f32 * 0.01, 0.1, 0.0]).unwrap())
            .speed_kms(Float::parse("42.5").unwrap())
            .fuel_pct(90 - minute)
            .recorded_at(UtcDateTime::parse(&format!("2187-02-01T06:{minute:02}:00Z")).unwrap())
            .source_ip(Inet::parse("10.0.2.1").unwrap())
            .await
            .unwrap();
    }
    assert_eq!(telemetry_rows(&h, "telemetry_pings").await, 25);

    // Pings carry the transponder's own ship; it can't report for another.
    let pings = TelemetryPing::query(&transponder).all().await.unwrap();
    assert!(pings.iter().all(|ping| ping.ship_id == ship.id));

    // Page through the track, oldest first.
    let mut minutes = Vec::new();
    let mut after = None;
    loop {
        let page = TelemetryPing::query(&transponder)
            .sort_by(TelemetryPing::recorded_at, false)
            .page_keyset(10, after.as_deref(), None)
            .await
            .unwrap();
        minutes.extend(page.results.iter().map(|ping| ping.fuel_pct));
        if !page.has_more {
            break;
        }
        after = page.after.clone();
    }
    assert_eq!(minutes, (66..=90).rev().collect::<Vec<_>>());

    // A bad key gets nothing, and crew can't pose as a transponder.
    assert!(matches!(h.app.transponder("lgx_not-a-key").await, Err(Error::Forbidden)));
    let crew = ash_core::Context::new(h.app.registry()).with_actor(h.ctx("Yuri").actor.clone().unwrap());
    let err = TelemetryPing::report(&crew)
        .position(Vector::new([0.0, 0.0, 0.0]).unwrap())
        .speed_kms(Float::parse("1").unwrap())
        .fuel_pct(50)
        .recorded_at(UtcDateTime::parse("2187-02-01T07:00:00Z").unwrap())
        .source_ip(Inet::parse("10.0.2.1").unwrap())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden), "{err:?}");
    // Fuel is a percentage.
    let err = TelemetryPing::report(&transponder)
        .position(Vector::new([0.0, 0.0, 0.0]).unwrap())
        .speed_kms(Float::parse("1").unwrap())
        .fuel_pct(140)
        .recorded_at(UtcDateTime::parse("2187-02-01T07:00:00Z").unwrap())
        .source_ip(Inet::parse("10.0.2.1").unwrap())
        .await;
    assert!(err.is_err(), "the telemetry database checks the fuel range");
}
on_every_backend!(transponders_report_positions);

async fn fleet_actions_are_audited<D: FleetDb>(h: Harness<D>) {
    let before = telemetry_rows(&h, "ops_events").await;
    let grace = h.ctx("Grace").clone().with_metadata("client_ip", "203.0.113.9");
    let contract = Contract::book(&grace)
        .external_ref("AUDIT-1")
        .shipper_email(CiString::parse("grace@helios-freight.example").unwrap())
        .origin_port_id(h.sol.port("KSC").id)
        .destination_port_id(h.sol.port("OLY").id)
        .freight_credits(1_000)
        .insured_value(Decimal::parse("1").unwrap())
        .deliver_by(Date::parse("2187-01-01").unwrap())
        .await
        .unwrap();
    assert_eq!(telemetry_rows(&h, "ops_events").await, before + 1);

    let audit = ash_core::Context::new(h.app.registry());
    let event = OpsEvent::query(&audit)
        .filter(OpsEvent::record_id.eq(contract.id))
        .one()
        .await
        .unwrap();
    assert_eq!(event.resource, "Contract");
    assert_eq!(event.action, "book");
    assert_eq!(event.line.as_deref(), Some(lagrange::seed::HELIOS));
    assert_eq!(event.actor_id, Some(h.sol.crew("Grace").id));
    assert_eq!(event.client_ip, Some(Inet::parse("203.0.113.9").unwrap()));

    // A workflow that fails leaves no audit trail.
    let before = telemetry_rows(&h, "ops_events").await;
    let err = lagrange::ops::plan_voyage(
        h.ctx("Ada"),
        h.sol.ship("Behemoth"),
        h.sol.port("KSC").id,
        h.sol.port("OLY").id,
        Date::parse("2187-01-01").unwrap(),
    )
    .await;
    assert!(err.is_err());
    assert_eq!(telemetry_rows(&h, "ops_events").await, before);
}
on_every_backend!(fleet_actions_are_audited);
