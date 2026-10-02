//! A lived-in Austin to start from: the zones and hubs, a few dozen regular riders, the
//! fleet spread across town (some charging, a couple in the shop), the last two hours of
//! trips, and a few minutes of pulse so the charts have a history from the first frame.

use ash_core::{Context, DataLayer, Result, UtcDateTimeUsec};

use crate::city::City;
use crate::rng::Rng;
use crate::sim::fare_cents;
use crate::types::RiderTier;
use crate::{Cab, Depot, PulseSample, Rider, ServiceZone, Trip};

const FIRST_NAMES: &[&str] = &[
    "Priya", "Marcus", "Elena", "Jamal", "Sofia", "Diego", "Hannah", "Kenji", "Amara", "Luis",
    "Grace", "Omar", "Chloe", "Tariq", "Maya", "Wyatt", "Zoe", "Rafael", "Nina", "Caleb", "Ines",
    "Theo", "Ava", "Mateo", "Leah", "Ravi", "Isla", "Andre", "Freya", "Hugo", "Lena", "Owen",
    "Tessa", "Yusuf", "Mila", "Jonah", "Ruby", "Dante", "Esme", "Felix",
];
const LAST_INITIALS: &str = "ABCDEFGHJKLMNPRSTVWY";

/// What the night shift calls the cabs.
const NICKNAMES: &[&str] = &[
    "Bluebonnet",
    "Mesquite",
    "Pecan",
    "Grackle",
    "Armadillo",
    "Longhorn",
    "Live Oak",
    "Bat Bridge",
    "Barton",
    "Colorado",
    "Zilker",
    "Cedar",
    "Mopac",
    "Lady Bird",
    "Pedernales",
    "Brazos",
    "Lavaca",
    "Guadalupe",
    "San Jacinto",
    "Red River",
    "Rainey",
    "Congress",
    "Lamar",
    "Shoal",
    "Waller",
    "Bouldin",
    "Travis",
    "Hyde",
    "Tarrytown",
    "Oltorf",
    "Riverside",
    "Montopolis",
    "Pease",
    "Mayfield",
];

pub const FLEET_SIZE: usize = 34;

fn ago(minutes: f64) -> UtcDateTimeUsec {
    let at = chrono::Utc::now() - chrono::Duration::milliseconds((minutes * 60_000.0) as i64);
    UtcDateTimeUsec::parse(&at.to_rfc3339()).expect("a valid instant")
}

/// Fills `ctx` with Austin as the shift finds it.
pub async fn austin<D: DataLayer>(ctx: &Context<D>, city: &City, seed: u64) -> Result<()> {
    let mut rng = Rng::seeded(seed);

    let mut zones = Vec::new();
    for spec in &city.zones {
        let zone = ServiceZone::chart(ctx)
            .code(spec.code.clone())
            .name(spec.name.clone())
            .lng(spec.center[0])
            .lat(spec.center[1])
            .radius_m(spec.radius_m)
            .base_demand(spec.base_demand)
            .surge(1.0)
            .event_boost(1.0)
            .await?;
        zones.push(zone);
    }

    let mut depots = Vec::new();
    for hub in &city.depots {
        let depot = Depot::open(ctx)
            .code(hub.code.clone())
            .name(hub.name.clone())
            .lng(hub.at[0])
            .lat(hub.at[1])
            .stalls(hub.stalls.unwrap_or(8))
            .await?;
        depots.push((hub.code.clone(), depot));
    }

    let mut riders = Vec::new();
    for i in 0..96 {
        let name = FIRST_NAMES[i % FIRST_NAMES.len()];
        let initial = LAST_INITIALS.as_bytes()[rng.below(LAST_INITIALS.len())] as char;
        let tier = match rng.unit() {
            r if r < 0.06 => RiderTier::Founder,
            r if r < 0.32 => RiderTier::Plus,
            _ => RiderTier::Standard,
        };
        let rider = Rider::sign_up(ctx)
            .display_name(format!("{name} {initial}."))
            .tier(tier)
            .rating(((rng.between(4.3, 5.0)) * 100.0).round() / 100.0)
            .phone_last4(format!("{:04}", rng.below(10_000)))
            .assisted_boarding(rng.chance(0.07))
            .await?;
        riders.push(rider);
    }

    let mut cabs = Vec::new();
    for (i, nickname) in NICKNAMES.iter().enumerate().take(FLEET_SIZE) {
        let (stop, depot) = if !(5..FLEET_SIZE - 2).contains(&i) {
            // Charging, or in the shop: at a hub.
            let (code, depot) = &depots[i % depots.len()];
            (city.stop(code), depot)
        } else {
            let place = rng.pick(&city.places);
            let depot = &depots
                .iter()
                .min_by(|a, b| {
                    crate::city::metres_between(city.stop(&a.0).at, place.at)
                        .total_cmp(&crate::city::metres_between(city.stop(&b.0).at, place.at))
                })
                .expect("hubs")
                .1;
            (place, depot)
        };
        let battery = if i < 5 {
            30 + rng.below(30) as i64
        } else {
            45 + rng.below(54) as i64
        };
        let cab = Cab::commission(ctx)
            .call_sign(format!("CC-{:04}", 101 + i))
            .nickname(nickname.to_string())
            .vin(format!("7SAYC{:012}", 4_810_220 + i * 37))
            .software(
                if i % 7 == 0 {
                    "FSD v14.3.0 (beta ring)"
                } else {
                    "FSD v14.2.1"
                }
                .to_string(),
            )
            .depot_id(depot.id)
            .lng(stop.at[0] + rng.between(-0.00025, 0.00025))
            .lat(stop.at[1] + rng.between(-0.0002, 0.0002))
            .heading_deg(rng.below(360) as i64)
            .speed_kph(0)
            .battery_pct(battery)
            .range_km((battery as f64 * 4.6) as i64)
            .odometer_km((rng.between(2_000.0, 31_000.0) * 10.0).round() / 10.0)
            .cabin_temp_c(21.5)
            .await?;
        let cab = if i < 5 {
            cab.recall_on(ctx).await?.plug_in_on(ctx).await?
        } else if i >= FLEET_SIZE - 2 {
            cab.ground_on(ctx).await?
        } else {
            cab
        };
        cabs.push(cab);
    }

    // The last two hours.
    let mut code = 48_210u64;
    let weights: Vec<f64> = city.zones.iter().map(|z| z.base_demand).collect();
    let mut history = Vec::new();
    for _ in 0..132 {
        let zone_index = rng.weighted(&weights);
        let zone = &zones[zone_index];
        let pickups: Vec<_> = city.places_in(&city.zones[zone_index].code).collect();
        let pickup = *rng.pick(&pickups);
        let others: Vec<_> = city
            .places
            .iter()
            .filter(|p| p.code != pickup.code)
            .collect();
        let dropoff = *rng.pick(&others);
        let route = city.route(&pickup.code, &dropoff.code);
        let surge = if rng.chance(0.2) {
            1.2 + (rng.below(5) as f64) / 10.0
        } else {
            1.0
        };
        history.push((rng.between(4.0, 125.0), zone, pickup, dropoff, route, surge));
    }
    history.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (minutes_ago, zone, pickup, dropoff, route, surge) in history {
        code += 1;
        let rider = rng.pick(&riders);
        let cab = &cabs[5 + rng.below(FLEET_SIZE - 7)];
        let trip = Trip::request(ctx)
            .code(format!("R-{code}"))
            .rider_id(rider.id)
            .zone_id(zone.id)
            .pickup_name(pickup.name.clone())
            .pickup_lng(pickup.at[0])
            .pickup_lat(pickup.at[1])
            .dropoff_name(dropoff.name.clone())
            .dropoff_lng(dropoff.at[0])
            .dropoff_lat(dropoff.at[1])
            .ride_polyline(route.encode())
            .distance_m(route.distance_m as i64)
            .duration_s(route.duration_s as i64)
            .surge(surge)
            .fare_cents(fare_cents(route.distance_m, route.duration_s, surge))
            .requested_at(ago(minutes_ago))
            .await?;
        if rng.chance(0.05) {
            trip.cancel_on(ctx)
                .cancelled_at(ago(minutes_ago - 1.5))
                .cancel_reason("Rider cancelled".to_string())
                .await?;
            continue;
        }
        let wait = rng.between(1.5, 6.5);
        let ride = route.duration_s / 60.0;
        trip.assign_on(ctx)
            .cab_id(cab.id)
            .assigned_at(ago(minutes_ago - 0.2))
            .pickup_eta_at(ago(minutes_ago - wait))
            .await?
            .arrive_on(ctx)
            .arrived_at(ago(minutes_ago - wait))
            .await?
            .board_on(ctx)
            .picked_up_at(ago(minutes_ago - wait - 0.6))
            .dropoff_eta_at(ago((minutes_ago - wait - 0.6 - ride).max(0.5)))
            .await?
            .complete_on(ctx)
            .completed_at(ago((minutes_ago - wait - 0.6 - ride).max(0.5)))
            .rating(if rng.chance(0.8) { 5 } else { 4 })
            .await?;
    }

    // A few minutes of pulse, so the sparklines start with a history.
    let completed: Vec<Trip> = Trip::query(ctx)
        .filter(ash_core::Filter::eq("status", "completed"))
        .all()
        .await?;
    let revenue: i64 = completed.iter().map(|t| t.fare_cents).sum();
    let mut busy = 14.0;
    for i in (1..=60).rev() {
        busy = (busy + rng.between(-1.5, 1.5)).clamp(8.0, 22.0);
        let on_trip = (busy * 0.62).round() as i64;
        let dispatched = busy.round() as i64 - on_trip;
        PulseSample::record(ctx)
            .recorded_at(ago(i as f64 * 5.0 / 60.0))
            .available(FLEET_SIZE as i64 - 7 - busy.round() as i64)
            .dispatched(dispatched)
            .on_trip(on_trip)
            .returning(rng.below(2) as i64)
            .charging(5)
            .maintenance(2)
            .waiting(rng.below(4) as i64)
            .completed_today(completed.len() as i64 - (i as i64) / 12)
            .revenue_cents_today(revenue - (i as i64) * 180)
            .avg_wait_s(240 + rng.below(120) as i64)
            .utilization_pct((busy * 100.0 / (FLEET_SIZE as f64 - 2.0)).round() as i64)
            .avg_battery_pct(64 + rng.below(6) as i64)
            .await?;
    }
    Ok(())
}
