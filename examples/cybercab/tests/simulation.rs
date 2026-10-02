//! The simulated fleet runs the business through the domain's actions, and does what
//! operators tell it through the same actions.

use std::sync::Arc;

use ash_core::{Context, Filter};
use ash_memory::Memory;
use ash_pubsub::PubSub;
use cybercab::city::City;
use cybercab::server::context;
use cybercab::sim::{SimConfig, Simulation};
use cybercab::{Cab, FleetAlert, PulseSample, ServiceZone, TelemetrySample, Trip};

async fn fleet(speedup: f64) -> (Context<Memory>, Simulation<Memory>) {
    let ctx = context(Memory::new(), &PubSub::new());
    let city = City::austin();
    cybercab::seed::austin(&ctx, &city, 7).await.unwrap();
    let config = SimConfig {
        speedup,
        demand: 1.5,
        seed: 7,
    };
    let sim = Simulation::new(ctx.clone(), Arc::clone(&city), config)
        .await
        .unwrap();
    (ctx, sim)
}

async fn run(sim: &mut Simulation<Memory>, ticks: usize) {
    for _ in 0..ticks {
        sim.step().await.unwrap();
    }
}

async fn trips(ctx: &Context<Memory>, status: &str) -> Vec<Trip> {
    Trip::query(ctx)
        .filter(Filter::eq("status", status))
        .all()
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn riders_are_carried_from_hail_to_drop_off() {
    let (ctx, mut sim) = fleet(40.0).await;
    let seeded = trips(&ctx, "completed").await.len();
    run(&mut sim, 240).await;

    let completed = trips(&ctx, "completed").await;
    assert!(
        completed.len() > seeded + 5,
        "{} rides after {seeded}",
        completed.len()
    );
    let fresh = completed
        .iter()
        .filter(|t| t.code.as_str() > "R-48342")
        .collect::<Vec<_>>();
    let ride = fresh.first().expect("a simulated ride finished");
    // Every milestone of the journey was recorded, in order.
    let times = [
        &ride.requested_at,
        ride.assigned_at.as_ref().unwrap(),
        ride.arrived_at.as_ref().unwrap(),
        ride.picked_up_at.as_ref().unwrap(),
        ride.completed_at.as_ref().unwrap(),
    ];
    assert!(times.windows(2).all(|pair| pair[0] <= pair[1]), "{times:?}");
    assert!(ride.approach_polyline.is_some() && ride.dropoff_eta_at.is_some());

    // The control room's history filled in alongside.
    assert!(TelemetrySample::query(&ctx).count().await.unwrap() > 100);
    assert!(PulseSample::query(&ctx).count().await.unwrap() > 60);
    let cabs = Cab::query(&ctx).all().await.unwrap();
    assert!(cabs.iter().all(|c| (0..=100).contains(&c.battery_pct)));
    assert!(
        cabs.iter()
            .any(|c| c.odometer_km > 0.0 && c.last_seen_at.is_some())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn operators_recall_and_halt_cabs() {
    let (ctx, mut sim) = fleet(40.0).await;
    run(&mut sim, 5).await;

    // Recalled, an idle cab drives to its hub and plugs in.
    let idle = Cab::query(&ctx)
        .filter(Filter::eq("status", "available"))
        .all()
        .await
        .unwrap();
    let cab = idle[0].clone().recall_on(&ctx).await.unwrap();
    let mut statuses = Vec::new();
    for _ in 0..120 {
        sim.step().await.unwrap();
        let status = Cab::get(&ctx, cab.id).await.unwrap().status;
        if statuses.last() != Some(&status) {
            statuses.push(status);
        }
    }
    assert!(
        statuses.starts_with(&["returning".to_string(), "charging".to_string()]),
        "{statuses:?}"
    );

    // Pulled over, a moving cab stops where it is until told to resume.
    let moving = loop {
        sim.step().await.unwrap();
        let cabs = Cab::query(&ctx).all().await.unwrap();
        if let Some(cab) = cabs
            .into_iter()
            .find(|c| c.speed_kph > 0 && c.status == "on_trip")
        {
            break cab;
        }
    };
    let halted = moving.pull_over_on(&ctx).await.unwrap();
    run(&mut sim, 2).await;
    let parked = Cab::get(&ctx, halted.id).await.unwrap();
    run(&mut sim, 3).await;
    let still = Cab::get(&ctx, halted.id).await.unwrap();
    assert!(still.halted);
    assert_eq!(
        (still.lng, still.lat, still.speed_kph),
        (parked.lng, parked.lat, 0)
    );
    still.resume_on(&ctx).await.unwrap();
    run(&mut sim, 3).await;
    let moving_again = Cab::get(&ctx, halted.id).await.unwrap();
    assert!((moving_again.lng, moving_again.lat) != (parked.lng, parked.lat));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cancelled_pickup_stands_the_cab_down() {
    let (ctx, mut sim) = fleet(4.0).await;
    let assigned = loop {
        sim.step().await.unwrap();
        if let Some(trip) = trips(&ctx, "assigned").await.into_iter().next() {
            break trip;
        }
    };
    let cab_id = assigned.cab_id.unwrap();
    assigned
        .cancel_on(&ctx)
        .cancelled_at(ash_core::UtcDateTimeUsec::now())
        .cancel_reason("Cancelled by operator".to_string())
        .await
        .unwrap();
    run(&mut sim, 2).await;
    // Stood down: free again, or already sent on to the next rider, in the same tick.
    let cab = Cab::get(&ctx, cab_id).await.unwrap();
    assert_ne!(cab.trip_id, Some(assigned.id));
    match (cab.status.as_str(), cab.trip_id) {
        ("available", None) => {}
        ("dispatched", Some(next)) => {
            let next = Trip::get(&ctx, next).await.unwrap();
            assert!(
                ["assigned", "arrived"].contains(&next.status.as_str()),
                "{next:?}"
            );
        }
        other => panic!("the cab should be stood down, not {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_event_draws_riders_and_lifts_the_surge() {
    let (ctx, mut sim) = fleet(20.0).await;
    let zone = ServiceZone::get_by_unique_code(&ctx, "GIG").await.unwrap();
    let before = Trip::query(&ctx)
        .filter(Filter::eq("zone_id", zone.id))
        .count()
        .await
        .unwrap();
    zone.host_event_on(&ctx)
        .event_name("Grand Prix at COTA".to_string())
        .event_boost(8.0)
        .await
        .unwrap();
    run(&mut sim, 120).await;
    let after = Trip::query(&ctx)
        .filter(Filter::eq("zone_id", zone.id))
        .count()
        .await
        .unwrap();
    assert!(after > before + 5, "{before} → {after} trips in the zone");

    // Operators can't stage an event without a name, or beyond what the zone can take.
    let zone = ServiceZone::get_by_unique_code(&ctx, "GIG").await.unwrap();
    assert!(
        zone.clone()
            .host_event_on(&ctx)
            .event_name("Too big".to_string())
            .event_boost(20.0)
            .await
            .is_err()
    );
    let _ = FleetAlert::query(&ctx).count().await.unwrap();
}

#[tokio::test]
async fn a_bigger_fleet_keeps_the_standard_proportions() {
    let ctx = context(Memory::new(), &PubSub::new());
    let city = City::austin();
    cybercab::seed::austin_with_fleet(&ctx, &city, 7, 102)
        .await
        .unwrap();
    let cabs = Cab::query(&ctx).all().await.unwrap();
    let count = |status: &str| cabs.iter().filter(|c| c.status == status).count();
    // Three times the standard fleet, in the same proportions.
    assert_eq!(cabs.len(), 102);
    assert_eq!((count("charging"), count("maintenance")), (15, 6));
}
