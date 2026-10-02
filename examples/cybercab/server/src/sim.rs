//! The fleet, simulated. Riders hail cabs across Austin; a dispatcher assigns the nearest
//! healthy cab; cabs drive real streets to the pickup, wait at the curb, carry the rider,
//! and head back to a hub to charge when they run low. Now and then something happens
//! that a person should look at.
//!
//! The simulation owns nothing. Every change goes through the domain's actions, as it
//! would from a real fleet, so the control room sees it live, and operator commands
//! (recall, pull over, take out of service, cancel a trip) reach it the same way: it
//! reads the domain every tick and does what it says.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use ash_core::{
    BulkCreateOptions, BulkDestroyOptions, BulkUpdateOptions, Context, DataLayer, Error, FieldMap,
    Filter, Resource, Result, UtcDateTimeUsec, Value,
};
use uuid::Uuid;

use crate::city::{City, Point, Route, metres_between};
use crate::rng::Rng;
use crate::types::{AlertKind, Severity};
use crate::{Cab, FleetAlert, PulseSample, Rider, ServiceZone, TelemetrySample, Trip};

/// How the simulation runs.
#[derive(Clone, Debug)]
pub struct SimConfig {
    /// Simulated seconds per real second: cabs drive this many times faster than life.
    pub speedup: f64,
    /// Scales how often riders hail a cab.
    pub demand: f64,
    pub seed: u64,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            speedup: 8.0,
            demand: 1.0,
            seed: 0xCAB5,
        }
    }
}

const ACTIVE_TRIP_STATES: [&str; 4] = ["requested", "assigned", "arrived", "riding"];
/// Ticks a rider waits for a cab before giving up.
const PATIENCE_TICKS: u32 = 150;
const LOW_BATTERY_PCT: i64 = 25;
const CHARGED_PCT: i64 = 92;

enum Purpose {
    Pickup(Uuid),
    Ride(Uuid),
    Hub,
}

struct Leg {
    route: Arc<Route>,
    travelled: f64,
    to: String,
    purpose: Purpose,
    /// How briskly this cab drives this leg, around 1.
    pace: f64,
}

/// What the simulation knows about a cab beyond what the domain holds.
struct Motion {
    /// The stop it's at, or last left.
    stop: String,
    leg: Option<Leg>,
    /// Waiting out an obstruction.
    held_ticks: u32,
    /// Waiting at the curb for a rider to board.
    boarding: Option<(Uuid, u32)>,
    /// Open obstruction alert, cleared when it moves on.
    obstruction: Option<Uuid>,
    low_battery_alerted: bool,
    ticks_since_report: u32,
    ticks_since_sample: u32,
    /// A small offset so cabs waiting at the same stop don't stack on the map.
    parking: Point,
}

/// A trip's stops, which only the simulation needs.
struct TripPlan {
    pickup: String,
    dropoff: String,
    waited_ticks: u32,
    assisted: bool,
}

pub struct Simulation<D: DataLayer> {
    ctx: Context<D>,
    city: Arc<City>,
    config: SimConfig,
    rng: Rng,
    motion: HashMap<Uuid, Motion>,
    plans: HashMap<Uuid, TripPlan>,
    riders: Vec<(Uuid, bool)>,
    tick: u64,
    /// The fleet's size against the standard 34 cabs: a bigger fleet serves a busier city.
    fleet_scale: f64,
    next_trip_code: u64,
    /// Seconds from request to pickup over the most recent pickups.
    recent_waits: VecDeque<i64>,
    /// This tick's position reports, written together once every cab has moved.
    reports: Vec<(Cab, FieldMap)>,
    /// This tick's telemetry samples, recorded together likewise.
    samples: Vec<FieldMap>,
}

impl<D: DataLayer> Simulation<D> {
    /// A simulation of the fleet `ctx` holds, as `seed::sol_city` left it.
    pub async fn new(ctx: Context<D>, city: Arc<City>, config: SimConfig) -> Result<Self> {
        let mut rng = Rng::seeded(config.seed);
        let riders = Rider::query(&ctx)
            .all()
            .await?
            .into_iter()
            .map(|rider| (rider.id, rider.assisted_boarding))
            .collect();
        let mut motion = HashMap::new();
        for cab in Cab::query(&ctx).all().await? {
            let stop = city
                .places
                .iter()
                .chain(&city.depots)
                .min_by(|a, b| {
                    metres_between(a.at, [cab.lng, cab.lat])
                        .total_cmp(&metres_between(b.at, [cab.lng, cab.lat]))
                })
                .map(|stop| stop.code.clone())
                .expect("the city has stops");
            let parking = [rng.between(-0.00025, 0.00025), rng.between(-0.0002, 0.0002)];
            motion.insert(
                cab.id,
                Motion {
                    stop,
                    leg: None,
                    held_ticks: 0,
                    boarding: None,
                    obstruction: None,
                    low_battery_alerted: false,
                    ticks_since_report: 0,
                    ticks_since_sample: 0,
                    parking,
                },
            );
        }
        let fleet_scale = (motion.len() as f64 / crate::seed::FLEET_SIZE as f64).max(1.0);
        let next_trip_code = 48_210 + Trip::query(&ctx).count().await? as u64;
        Ok(Self {
            ctx,
            city,
            config,
            rng,
            motion,
            plans: HashMap::new(),
            riders,
            tick: 0,
            fleet_scale,
            next_trip_code,
            recent_waits: VecDeque::new(),
            reports: Vec::new(),
            samples: Vec::new(),
        })
    }

    /// Runs forever, one tick a second.
    pub async fn run(mut self) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if let Err(err) = self.step().await {
                eprintln!("simulation tick {}: {err}", self.tick);
            }
        }
    }

    /// One second of the fleet.
    pub async fn step(&mut self) -> Result<()> {
        self.tick += 1;
        let cabs = Cab::query(&self.ctx).all().await?;
        let active = Trip::query(&self.ctx)
            .filter(Filter::in_list("status", ACTIVE_TRIP_STATES))
            .sort(Trip::requested_at)
            .all()
            .await?;

        self.follow_operators(&cabs, &active).await?;
        self.hail().await?;
        self.dispatch(&cabs, &active).await?;
        for cab in &cabs {
            if let Err(err) = self.drive(cab).await {
                eprintln!("simulation: {} {err}", cab.call_sign);
            }
        }
        self.send_reports().await?;
        if self.tick.is_multiple_of(5) {
            self.measure_zones(&active).await?;
            self.take_pulse().await?;
        }
        if self.tick.is_multiple_of(10) {
            self.settle_alerts().await?;
        }
        if self.tick.is_multiple_of(30) {
            self.prune().await?;
        }
        Ok(())
    }

    fn now(&self) -> UtcDateTimeUsec {
        UtcDateTimeUsec::now()
    }

    /// Simulated seconds from now, in real time.
    fn after(&self, simulated_s: f64) -> UtcDateTimeUsec {
        let real = simulated_s / self.config.speedup;
        let at = chrono::Utc::now() + chrono::Duration::milliseconds((real * 1000.0) as i64);
        UtcDateTimeUsec::parse(&at.to_rfc3339()).expect("a valid instant")
    }

    /// Does what operators asked: cabs recalled to a hub, grounded or released, and
    /// trips cancelled out from under the cab fetching them.
    async fn follow_operators(&mut self, cabs: &[Cab], active: &[Trip]) -> Result<()> {
        for cab in cabs {
            let Some(motion) = self.motion.get_mut(&cab.id) else {
                continue;
            };
            match cab.status.as_str() {
                "maintenance" => {
                    motion.leg = None;
                    motion.boarding = None;
                }
                "returning"
                    if !matches!(motion.leg.as_ref().map(|l| &l.purpose), Some(Purpose::Hub)) =>
                {
                    let from = position_of(&self.city, motion);
                    let hub = self.city.nearest_depot(from).code.clone();
                    let route = self.city.route(&motion.stop, &hub);
                    motion.leg = Some(Leg {
                        route,
                        travelled: 0.0,
                        to: hub,
                        purpose: Purpose::Hub,
                        pace: 1.0,
                    });
                }
                "dispatched" => {
                    // The trip it was fetching was cancelled.
                    let fetching = cab
                        .trip_id
                        .is_some_and(|id| active.iter().any(|t| t.id == id));
                    if !fetching {
                        motion.leg = None;
                        cab.clone()
                            .stand_down_on(&self.ctx)
                            .trip_id(None::<Uuid>)
                            .await?;
                    }
                }
                _ => {}
            }
        }
        // Plans for trips that are over.
        self.plans
            .retain(|id, _| active.iter().any(|t| t.id == *id));
        Ok(())
    }

    /// Riders hail cabs, more where demand is high and where an event is on.
    async fn hail(&mut self) -> Result<()> {
        let zones = ServiceZone::query(&self.ctx).all().await?;
        for zone in zones {
            let boost = if zone.event_name.is_some() {
                zone.event_boost.max(1.0)
            } else {
                1.0
            };
            // Requests a simulated minute, spread over this tick's simulated seconds.
            let per_tick =
                zone.base_demand * 0.27 * boost * self.config.demand * self.config.speedup / 60.0;
            let requests = hails(per_tick, self.fleet_scale, self.rng.unit());
            if requests == 0 {
                continue;
            }
            let pickups: Vec<String> = self
                .city
                .places_in(&zone.code)
                .map(|p| p.code.clone())
                .collect();
            if pickups.is_empty() {
                continue;
            }
            for _ in 0..requests {
                let pickup = self.rng.pick(&pickups).clone();
                let dropoff = self.pick_destination(&pickup);
                self.request(&zone, &pickup, &dropoff).await?;
            }
        }
        Ok(())
    }

    fn pick_destination(&mut self, from: &str) -> String {
        let places: Vec<&crate::city::Stop> =
            self.city.places.iter().filter(|p| p.code != from).collect();
        let weights: Vec<f64> = places
            .iter()
            .map(|p| match p.code.as_str() {
                "AUSA" => 2.2,
                "CON6" | "RAI" | "SOCO" | "DOMN" => 1.6,
                "COTA" | "GIGA" => 0.4,
                _ => 1.0,
            })
            .collect();
        places[self.rng.weighted(&weights)].code.clone()
    }

    async fn request(&mut self, zone: &ServiceZone, pickup: &str, dropoff: &str) -> Result<()> {
        let route = self.city.route(pickup, dropoff);
        let (rider_id, assisted) = *self.rng.pick(&self.riders);
        let (from, to) = (self.city.stop(pickup), self.city.stop(dropoff));
        let fare = fare_cents(route.distance_m, route.duration_s, zone.surge);
        self.next_trip_code += 1;
        let trip = Trip::request(&self.ctx)
            .code(format!("R-{}", self.next_trip_code))
            .rider_id(rider_id)
            .zone_id(zone.id)
            .pickup_name(from.name.clone())
            .pickup_lng(from.at[0])
            .pickup_lat(from.at[1])
            .dropoff_name(to.name.clone())
            .dropoff_lng(to.at[0])
            .dropoff_lat(to.at[1])
            .ride_polyline(route.encode())
            .distance_m(route.distance_m as i64)
            .duration_s(route.duration_s as i64)
            .surge(zone.surge)
            .fare_cents(fare)
            .requested_at(self.now())
            .await?;
        self.plans.insert(
            trip.id,
            TripPlan {
                pickup: pickup.to_string(),
                dropoff: dropoff.to_string(),
                waited_ticks: 0,
                assisted,
            },
        );
        Ok(())
    }

    /// Assigns each waiting rider the nearest healthy cab, oldest request first. A rider
    /// who waits too long gives up.
    async fn dispatch(&mut self, cabs: &[Cab], active: &[Trip]) -> Result<()> {
        let mut free: Vec<&Cab> = cabs
            .iter()
            .filter(|cab| {
                cab.status == "available" && !cab.halted && cab.battery_pct >= LOW_BATTERY_PCT
            })
            .collect();
        for trip in active.iter().filter(|t| t.status == "requested") {
            let Some(plan) = self.plans.get_mut(&trip.id) else {
                continue;
            };
            plan.waited_ticks += 1;
            let (pickup, waited) = (plan.pickup.clone(), plan.waited_ticks);
            let pickup_at = self.city.stop(&pickup).at;
            let nearest = free
                .iter()
                .enumerate()
                .filter_map(|(i, cab)| {
                    self.motion
                        .get(&cab.id)
                        .map(|m| (i, position_of(&self.city, m)))
                })
                .min_by(|(_, a), (_, b)| {
                    metres_between(*a, pickup_at).total_cmp(&metres_between(*b, pickup_at))
                });
            let Some((index, _)) = nearest else {
                if waited > PATIENCE_TICKS {
                    trip.clone()
                        .cancel_on(&self.ctx)
                        .cancelled_at(self.now())
                        .cancel_reason("No cab nearby".to_string())
                        .await?;
                }
                continue;
            };
            let cab = free.remove(index);
            let motion = self.motion.get_mut(&cab.id).expect("every cab has motion");
            let approach = self.city.route(&motion.stop, &pickup);
            let eta = self.after(approach.duration_s);
            trip.clone()
                .assign_on(&self.ctx)
                .cab_id(cab.id)
                .approach_polyline(approach.encode())
                .assigned_at(self.now())
                .pickup_eta_at(eta)
                .await?;
            cab.clone().dispatch_on(&self.ctx).trip_id(trip.id).await?;
            let pace = self.rng.between(0.9, 1.1);
            let motion = self.motion.get_mut(&cab.id).expect("every cab has motion");
            motion.boarding = None;
            motion.leg = Some(Leg {
                route: approach,
                travelled: 0.0,
                to: pickup,
                purpose: Purpose::Pickup(trip.id),
                pace,
            });
        }
        Ok(())
    }

    /// Moves one cab a tick along, and reports what it's doing.
    async fn drive(&mut self, cab: &Cab) -> Result<()> {
        let Some(mut motion) = self.motion.remove(&cab.id) else {
            return Ok(());
        };
        let result = self.drive_with(cab, &mut motion).await;
        self.motion.insert(cab.id, motion);
        result
    }

    async fn drive_with(&mut self, cab: &Cab, motion: &mut Motion) -> Result<()> {
        let mut battery = cab.battery_pct;
        let mut odometer = cab.odometer_km;
        let mut speed_kph = 0;
        let mut heading = cab.heading_deg;
        let mut moved = false;

        // At the curb, a rider boards.
        if let Some((trip_id, ticks)) = motion.boarding {
            if ticks > 0 {
                motion.boarding = Some((trip_id, ticks - 1));
            } else {
                motion.boarding = None;
                self.board(cab, motion, trip_id).await?;
            }
        }

        if cab.status == "charging" {
            battery = (battery + 2).min(100);
            if battery >= CHARGED_PCT {
                cab.clone().unplug_on(&self.ctx).await?;
                motion.low_battery_alerted = false;
            }
        } else if let Some(leg) = motion.leg.as_mut()
            && !cab.halted
            && cab.status != "maintenance"
        {
            if motion.held_ticks > 0 {
                motion.held_ticks -= 1;
            } else {
                if let Some(alert) = motion.obstruction.take() {
                    self.auto_resolve(alert).await?;
                }
                let remaining = leg.route.distance_m - leg.travelled;
                // Pulling away and pulling in are slower than cruising.
                let ramp = (leg.travelled.min(remaining) / 180.0).clamp(0.35, 1.0);
                let mps = leg.route.speed_mps() * leg.pace * ramp * self.rng.between(0.85, 1.1);
                let step = (mps * self.config.speedup).min(remaining);
                leg.travelled += step;
                odometer += step / 1000.0;
                battery = (battery as f64 - step * 0.00045 - self.rng.between(0.0, 0.2))
                    .round()
                    .max(1.0) as i64;
                speed_kph = (mps * 3.6).round() as i64;
                let (_, bearing) = leg.route.position(leg.travelled);
                heading = bearing.round() as i64;
                moved = true;
                self.maybe_incident(cab, motion).await?;
            }
        }

        // Arrived?
        let arrived = motion
            .leg
            .as_ref()
            .is_some_and(|leg| leg.travelled >= leg.route.distance_m - 0.5);
        if arrived {
            let leg = motion.leg.take().expect("arrived on a leg");
            motion.stop = leg.to.clone();
            speed_kph = 0;
            match leg.purpose {
                Purpose::Pickup(trip_id) => {
                    let trip = Trip::get(&self.ctx, trip_id).await?;
                    trip.arrive_on(&self.ctx).arrived_at(self.now()).await?;
                    let assisted = self.plans.get(&trip_id).is_some_and(|p| p.assisted);
                    let wait = self.rng.below(4) as u32 + 3 + if assisted { 5 } else { 0 };
                    motion.boarding = Some((trip_id, wait));
                    if self.rng.chance(0.04) {
                        self.raise(
                            cab,
                            motion,
                            Some(trip_id),
                            AlertKind::DoorAjar,
                            Severity::Info,
                            "Rear door reported ajar at pickup",
                        )
                        .await?;
                    }
                }
                Purpose::Ride(trip_id) => self.drop_off(cab, motion, trip_id, battery).await?,
                Purpose::Hub => {
                    cab.clone().plug_in_on(&self.ctx).await?;
                }
            }
        }

        // Report: every tick on the move or when anything changed, now and then when parked.
        motion.ticks_since_report += 1;
        let changed = speed_kph != cab.speed_kph || battery != cab.battery_pct;
        if moved || arrived || changed || motion.ticks_since_report >= 10 {
            // The tick's copy will do: an update writes only what it changes, so this
            // doesn't undo anything done to the cab since the tick began.
            let at = position_of(&self.city, motion);
            let mut report = FieldMap::new();
            report.insert("lng".into(), Value::from(at[0]));
            report.insert("lat".into(), Value::from(at[1]));
            report.insert("heading_deg".into(), Value::from(heading));
            report.insert("speed_kph".into(), Value::from(speed_kph));
            report.insert("battery_pct".into(), Value::from(battery));
            let range = (battery as f64 * 4.6).round() as i64;
            report.insert("range_km".into(), Value::from(range));
            let odometer = (odometer * 10.0).round() / 10.0;
            report.insert("odometer_km".into(), Value::from(odometer));
            let cabin = 21.5 + self.rng.between(-0.6, 0.6);
            report.insert("cabin_temp_c".into(), Value::from(cabin));
            report.insert("last_seen_at".into(), Value::from(self.now()));
            self.reports.push((cab.clone(), report));
            motion.ticks_since_report = 0;
        }
        motion.ticks_since_sample += 1;
        if moved && motion.ticks_since_sample >= 3 {
            let at = position_of(&self.city, motion);
            let mut sample = FieldMap::new();
            sample.insert("cab_id".into(), Value::Uuid(cab.id));
            sample.insert("recorded_at".into(), Value::from(self.now()));
            sample.insert("lng".into(), Value::from(at[0]));
            sample.insert("lat".into(), Value::from(at[1]));
            sample.insert("speed_kph".into(), Value::from(speed_kph));
            sample.insert("battery_pct".into(), Value::from(battery));
            self.samples.push(sample);
            motion.ticks_since_sample = 0;
        }
        if battery < 20 && !motion.low_battery_alerted {
            motion.low_battery_alerted = true;
            self.raise(
                cab,
                motion,
                cab.trip_id,
                AlertKind::LowBattery,
                Severity::Warning,
                &format!("Battery at {battery}%"),
            )
            .await?;
        }
        Ok(())
    }

    /// Writes this tick's position reports and telemetry samples together: a bulk update
    /// and a bulk create, which Postgres runs as a few statements rather than a round trip
    /// and a commit for every cab.
    async fn send_reports(&mut self) -> Result<()> {
        let reports = std::mem::take(&mut self.reports);
        if !reports.is_empty() {
            let opts = BulkUpdateOptions::new()
                .return_records(false)
                .stop_on_error(false);
            let result = Cab::bulk_update_with_opts(&self.ctx, "report", reports, opts).await?;
            for error in result.errors {
                eprintln!("simulation: report {error}");
            }
        }
        let samples = std::mem::take(&mut self.samples);
        if !samples.is_empty() {
            let opts = BulkCreateOptions::new()
                .return_records(false)
                .stop_on_error(false);
            let result =
                TelemetrySample::bulk_create_with_opts(&self.ctx, "record", samples, opts).await?;
            for error in result.errors {
                eprintln!("simulation: sample {error}");
            }
        }
        Ok(())
    }

    async fn board(&mut self, cab: &Cab, motion: &mut Motion, trip_id: Uuid) -> Result<()> {
        let trip = Trip::get(&self.ctx, trip_id).await?;
        if trip.status != "arrived" {
            return Ok(());
        }
        let picked_up = self.now();
        let waited = chrono::DateTime::parse_from_rfc3339(picked_up.as_str())
            .ok()
            .zip(chrono::DateTime::parse_from_rfc3339(trip.requested_at.as_str()).ok())
            .map(|(a, b)| ((a - b).num_milliseconds() as f64 / 1000.0 * self.config.speedup) as i64)
            .unwrap_or(0);
        self.recent_waits.push_back(waited);
        if self.recent_waits.len() > 30 {
            self.recent_waits.pop_front();
        }
        let Some(plan) = self.plans.get(&trip_id) else {
            return Ok(());
        };
        let route = self.city.route(&plan.pickup, &plan.dropoff);
        trip.board_on(&self.ctx)
            .picked_up_at(picked_up)
            .dropoff_eta_at(self.after(route.duration_s))
            .await?;
        Cab::get(&self.ctx, cab.id)
            .await?
            .begin_ride_on(&self.ctx)
            .await?;
        motion.leg = Some(Leg {
            route,
            travelled: 0.0,
            to: plan.dropoff.clone(),
            purpose: Purpose::Ride(trip_id),
            pace: self.rng.between(0.92, 1.08),
        });
        Ok(())
    }

    async fn drop_off(
        &mut self,
        cab: &Cab,
        motion: &mut Motion,
        trip_id: Uuid,
        battery: i64,
    ) -> Result<()> {
        let rating = match self.rng.unit() {
            r if r < 0.78 => 5,
            r if r < 0.95 => 4,
            r if r < 0.99 => 3,
            _ => 2,
        };
        Trip::get(&self.ctx, trip_id)
            .await?
            .complete_on(&self.ctx)
            .completed_at(self.now())
            .rating(rating)
            .await?;
        let cab = Cab::get(&self.ctx, cab.id)
            .await?
            .finish_ride_on(&self.ctx)
            .trip_id(None::<Uuid>)
            .await?;
        if battery < LOW_BATTERY_PCT {
            let hub = self
                .city
                .nearest_depot(position_of(&self.city, motion))
                .code
                .clone();
            let route = self.city.route(&motion.stop, &hub);
            cab.recall_on(&self.ctx).await?;
            motion.leg = Some(Leg {
                route,
                travelled: 0.0,
                to: hub,
                purpose: Purpose::Hub,
                pace: 1.0,
            });
        }
        Ok(())
    }

    /// Now and then, something a person should look at.
    async fn maybe_incident(&mut self, cab: &Cab, motion: &mut Motion) -> Result<()> {
        let riding = cab.status == "on_trip";
        let roll = self.rng.unit();
        if roll < 0.0011 {
            motion.held_ticks = 6 + self.rng.below(8) as u32;
            let id = self
                .raise(
                    cab,
                    motion,
                    cab.trip_id,
                    AlertKind::Obstruction,
                    Severity::Warning,
                    "Stopped for an obstruction in the lane",
                )
                .await?;
            motion.obstruction = Some(id);
        } else if roll < 0.0019 {
            self.raise(
                cab,
                motion,
                cab.trip_id,
                AlertKind::HardBraking,
                Severity::Warning,
                "Hard braking event (0.6 g)",
            )
            .await?;
        } else if riding && roll < 0.0025 {
            self.raise(
                cab,
                motion,
                cab.trip_id,
                AlertKind::RiderAssist,
                Severity::Critical,
                "Rider pressed the assist button",
            )
            .await?;
        } else if roll < 0.0027 {
            self.raise(
                cab,
                motion,
                cab.trip_id,
                AlertKind::SensorDegraded,
                Severity::Info,
                "Front-left camera degraded: glare",
            )
            .await?;
        }
        Ok(())
    }

    async fn raise(
        &mut self,
        cab: &Cab,
        motion: &Motion,
        trip_id: Option<Uuid>,
        kind: AlertKind,
        severity: Severity,
        message: &str,
    ) -> Result<Uuid> {
        let at = position_of(&self.city, motion);
        let alert = FleetAlert::raise(&self.ctx)
            .cab_id(cab.id)
            .trip_id(trip_id)
            .kind(kind)
            .severity(severity)
            .message(format!("{} · {message}", cab.call_sign))
            .lng(at[0])
            .lat(at[1])
            .raised_at(self.now())
            .await?;
        Ok(alert.id)
    }

    async fn auto_resolve(&self, alert: Uuid) -> Result<()> {
        match FleetAlert::get(&self.ctx, alert).await {
            Ok(alert) if alert.status != "resolved" => {
                alert
                    .resolve_on(&self.ctx)
                    .resolved_at(self.now())
                    .handled_by("auto".to_string())
                    .await?;
                Ok(())
            }
            Ok(_) | Err(Error::NotFound) => Ok(()),
            Err(err) => Err(err),
        }
    }

    /// Alerts nobody touched clear themselves after a few minutes, except rider assists.
    async fn settle_alerts(&mut self) -> Result<()> {
        let stale = chrono::Utc::now() - chrono::Duration::minutes(3);
        let stale = UtcDateTimeUsec::parse(&stale.to_rfc3339()).expect("a valid instant");
        let open = FleetAlert::query(&self.ctx)
            .filter(
                Filter::eq("status", "open")
                    & Filter::lt("raised_at", stale.as_str())
                    & Filter::ne("kind", "rider_assist"),
            )
            .all()
            .await?;
        for alert in open {
            self.auto_resolve(alert.id).await?;
        }
        Ok(())
    }

    /// Riders waiting in each zone, and the surge they call for.
    async fn measure_zones(&mut self, active: &[Trip]) -> Result<()> {
        for zone in ServiceZone::query(&self.ctx).all().await? {
            let waiting = active
                .iter()
                .filter(|t| t.zone_id == zone.id && t.status == "requested")
                .count() as i64;
            let boost = if zone.event_name.is_some() {
                zone.event_boost
            } else {
                1.0
            };
            let surge = ((1.0 + waiting as f64 * 0.15 + (boost - 1.0) * 0.2).clamp(1.0, 3.0)
                * 10.0)
                .round()
                / 10.0;
            if zone.waiting != waiting || (zone.surge - surge).abs() > f64::EPSILON {
                zone.measure_on(&self.ctx)
                    .waiting(waiting)
                    .surge(surge)
                    .await?;
            }
        }
        Ok(())
    }

    async fn take_pulse(&mut self) -> Result<()> {
        let cabs = Cab::query(&self.ctx).all().await?;
        let count = |status: &str| cabs.iter().filter(|c| c.status == status).count() as i64;
        let in_service = cabs.len() as i64 - count("maintenance");
        let busy = count("dispatched") + count("on_trip");
        let completed = Trip::query(&self.ctx)
            .filter(Filter::eq("status", "completed"))
            .all()
            .await?;
        let waiting = Trip::query(&self.ctx)
            .filter(Filter::eq("status", "requested"))
            .count()
            .await? as i64;
        let avg_wait = if self.recent_waits.is_empty() {
            0
        } else {
            self.recent_waits.iter().sum::<i64>() / self.recent_waits.len() as i64
        };
        PulseSample::record(&self.ctx)
            .recorded_at(self.now())
            .available(count("available"))
            .dispatched(count("dispatched"))
            .on_trip(count("on_trip"))
            .returning(count("returning"))
            .charging(count("charging"))
            .maintenance(count("maintenance"))
            .waiting(waiting)
            .completed_today(completed.len() as i64)
            .revenue_cents_today(completed.iter().map(|t| t.fare_cents).sum::<i64>())
            .avg_wait_s(avg_wait)
            .utilization_pct(if in_service > 0 {
                busy * 100 / in_service
            } else {
                0
            })
            .avg_battery_pct(if cabs.is_empty() {
                0
            } else {
                cabs.iter().map(|c| c.battery_pct).sum::<i64>() / cabs.len() as i64
            })
            .await?;
        Ok(())
    }

    /// Keeps the history bounded: recent trails, pulse and finished trips.
    async fn prune(&mut self) -> Result<()> {
        let mut doomed = Vec::new();
        let samples = TelemetrySample::query(&self.ctx)
            .sort_desc(TelemetrySample::recorded_at)
            .all()
            .await?;
        let mut kept: HashMap<Uuid, usize> = HashMap::new();
        for sample in samples {
            let seen = kept.entry(sample.cab_id).or_default();
            *seen += 1;
            if *seen > 120 {
                doomed.push(sample.id);
            }
        }
        destroy::<TelemetrySample, D>(&self.ctx, &doomed).await?;

        let pulses = PulseSample::query(&self.ctx)
            .sort_desc(PulseSample::recorded_at)
            .offset(240)
            .all()
            .await?;
        destroy::<PulseSample, D>(&self.ctx, &pulses.iter().map(|p| p.id).collect::<Vec<_>>())
            .await?;

        let finished = Trip::query(&self.ctx)
            .filter(Filter::in_list("status", ["completed", "cancelled"]))
            .sort_desc(Trip::requested_at)
            .offset(400)
            .all()
            .await?;
        destroy::<Trip, D>(
            &self.ctx,
            &finished.iter().map(|t| t.id).collect::<Vec<_>>(),
        )
        .await?;
        Ok(())
    }
}

async fn destroy<R: Resource, D: DataLayer>(ctx: &Context<D>, ids: &[Uuid]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let action = R::DEF
        .actions
        .iter()
        .find(|a| a.kind == ash_core::ActionKind::Destroy)
        .map(|a| a.name)
        .unwrap_or("destroy");
    ash_core::bulk_destroy::<R, D>(
        ctx,
        action,
        ids,
        BulkDestroyOptions::default()
            .return_records(false)
            .stop_on_error(false),
    )
    .await?;
    Ok(())
}

/// Where a cab is: along its leg, or parked at its stop.
fn position_of(city: &City, motion: &Motion) -> Point {
    match &motion.leg {
        Some(leg) if leg.travelled > 0.0 => leg.route.position(leg.travelled).0,
        _ => {
            let at = city.stop(&motion.stop).at;
            [at[0] + motion.parking[0], at[1] + motion.parking[1]]
        }
    }
}

/// How many riders hail in a zone this tick, when `per_tick` hail there on average for
/// the standard fleet and `roll` is uniform in `[0, 1)`. The standard fleet sees at most
/// one a tick; a fleet `fleet_scale` times bigger sees proportionally more.
fn hails(per_tick: f64, fleet_scale: f64, roll: f64) -> usize {
    let expected = if fleet_scale > 1.0 {
        per_tick * fleet_scale
    } else {
        per_tick.min(0.9)
    };
    expected.floor() as usize + usize::from(roll < expected.fract())
}

/// $2.50 to start, $1.10 a kilometre and $0.25 a minute, times the surge; at least $6.
pub fn fare_cents(distance_m: f64, duration_s: f64, surge: f64) -> i64 {
    let base = 250.0 + 110.0 * distance_m / 1000.0 + 25.0 * duration_s / 60.0;
    ((base * surge).round() as i64).max(600)
}

#[cfg(test)]
mod tests {
    use super::hails;

    /// Hails over evenly spread rolls: what a zone sees on average a tick.
    fn average(per_tick: f64, fleet_scale: f64) -> f64 {
        let rolls = 10_000;
        (0..rolls)
            .map(|i| hails(per_tick, fleet_scale, i as f64 / rolls as f64))
            .sum::<usize>() as f64
            / rolls as f64
    }

    #[test]
    fn the_standard_fleet_sees_at_most_one_hail_a_zone_a_tick() {
        assert!((average(0.3, 1.0) - 0.3).abs() < 1e-3);
        assert!((average(2.0, 1.0) - 0.9).abs() < 1e-3);
        assert!((0..100).all(|i| hails(2.0, 1.0, i as f64 / 100.0) <= 1));
    }

    #[test]
    fn a_bigger_fleet_sees_proportionally_more() {
        assert!((average(0.3, 3.0) - 0.9).abs() < 1e-3);
        assert!((average(0.3, 10.0) - 3.0).abs() < 1e-3);
        assert!((average(0.45, 10.0) - 4.5).abs() < 1e-3);
    }
}
