//! How far the command center scales, measured end to end on a single machine.
//!
//! For each fleet size it seeds Austin, times the reads and writes the simulation leans
//! on, then runs the simulation and times its ticks against the one-second budget a
//! live fleet gives it. Separately, it measures fan-out: whole-fleet rounds of position
//! reports delivered over `/graphql/ws` to a number of subscribers, as wall displays and
//! consoles would receive them.
//!
//! ```bash
//! cargo bench -p cybercab --bench scale
//! cargo bench -p cybercab --bench scale -- --fleets 34,1000,10000 --subscribers 1,10,100
//! cargo bench -p cybercab --bench scale -- --json before.json
//! ```

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ash_core::{Context, Filter, Notification, Notifier, Result, UtcDateTimeUsec};
use ash_memory::Memory;
use ash_pubsub::PubSub;
use cybercab::city::City;
use cybercab::server::{context, router};
use cybercab::sim::{SimConfig, Simulation};
use cybercab::{Cab, TelemetrySample};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

struct Args {
    fleets: Vec<usize>,
    fanout_fleets: Vec<usize>,
    subscribers: Vec<usize>,
    warmup: usize,
    ticks: usize,
    rounds: usize,
    /// Fleet sizes projected to take longer than this to seed and simulate are skipped.
    budget: Duration,
    json: Option<String>,
}

fn args() -> Args {
    let mut args = Args {
        fleets: vec![34, 250, 1_000, 2_500, 5_000, 10_000],
        fanout_fleets: vec![34, 1_000, 5_000],
        subscribers: vec![1, 10, 100],
        warmup: 20,
        ticks: 20,
        rounds: 5,
        budget: Duration::from_secs(300),
        json: None,
    };
    let list = |v: String| {
        v.split(',')
            .map(|n| n.trim().parse().expect("a number"))
            .collect()
    };
    let mut raw = std::env::args().skip(1);
    while let Some(flag) = raw.next() {
        let mut value = || raw.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        match flag.as_str() {
            "--fleets" => args.fleets = list(value()),
            "--fanout-fleets" => args.fanout_fleets = list(value()),
            "--subscribers" => args.subscribers = list(value()),
            "--warmup" => args.warmup = value().parse().expect("a number"),
            "--ticks" => args.ticks = value().parse().expect("a number"),
            "--rounds" => args.rounds = value().parse().expect("a number"),
            "--budget" => args.budget = Duration::from_secs(value().parse().expect("seconds")),
            "--json" => args.json = Some(value()),
            // `cargo bench` passes `--bench`; anything else is cargo's too.
            _ => {}
        }
    }
    args
}

#[derive(Serialize, Default)]
struct Report {
    machine: String,
    fleets: Vec<FleetReport>,
    fanout: Vec<FanoutReport>,
}

#[derive(Serialize)]
struct FleetReport {
    fleet: usize,
    seed_s: f64,
    /// Median microseconds for one `Cab::get`.
    get_us: f64,
    /// Median milliseconds to read every cab.
    read_all_ms: f64,
    /// Median milliseconds to find a cab by its call sign.
    find_by_call_sign_ms: f64,
    /// Median microseconds for one `Cab.report` update.
    report_us: f64,
    /// Median microseconds to record a telemetry sample.
    record_us: f64,
    tick_ms: Spread,
    actions_per_tick: f64,
    actions_per_s: f64,
    busy_pct: i64,
}

#[derive(Serialize)]
struct FanoutReport {
    fleet: usize,
    subscribers: usize,
    /// Milliseconds to publish one report from every cab.
    publish_round_ms: Spread,
    expected: usize,
    delivered: usize,
    latency_ms: Spread,
    /// Subscriptions the server ended before the run did.
    ended: usize,
}

#[derive(Serialize, Clone, Copy, Default)]
struct Spread {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

impl Spread {
    fn of(mut values: Vec<f64>) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        Self {
            p50: at(0.5),
            p95: at(0.95),
            p99: at(0.99),
            max: values[values.len() - 1],
        }
    }
}

/// Counts every action that commits, whatever the resource.
#[derive(Debug, Default)]
struct Counter(AtomicU64);

impl Notifier for Counter {
    fn notify<'a>(
        &'a self,
        _notification: &'a Notification,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Box::pin(async { Ok(()) })
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn us(d: Duration) -> f64 {
    d.as_secs_f64() * 1e6
}

fn median(values: Vec<f64>) -> f64 {
    Spread::of(values).p50
}

async fn seeded(fleet: usize, pubsub: &PubSub) -> (Context<Memory>, Arc<City>, Duration) {
    let ctx = context(Memory::new(), pubsub);
    let city = City::austin();
    let started = Instant::now();
    cybercab::seed::austin_with_fleet(&ctx, &city, 7, fleet)
        .await
        .expect("seeds");
    (ctx, city, started.elapsed())
}

async fn fleet(fleet: usize, warmup: usize, ticks: usize) -> FleetReport {
    let pubsub = PubSub::new();
    let (ctx, city, seed) = seeded(fleet, &pubsub).await;
    let cabs = Cab::query(&ctx).all().await.expect("reads");

    let samples = 200.min(fleet * 4);
    let mut get = Vec::new();
    for cab in cabs.iter().cycle().take(samples) {
        let started = Instant::now();
        Cab::get(&ctx, cab.id).await.expect("gets");
        get.push(us(started.elapsed()));
    }
    let mut read_all = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        Cab::query(&ctx).all().await.expect("reads");
        read_all.push(ms(started.elapsed()));
    }
    let mut find = Vec::new();
    for cab in cabs.iter().take(5) {
        let started = Instant::now();
        Cab::query(&ctx)
            .filter(Filter::eq("call_sign", cab.call_sign.as_str()))
            .one()
            .await
            .expect("finds");
        find.push(ms(started.elapsed()));
    }
    let mut report = Vec::new();
    for cab in cabs.iter().cycle().take(samples) {
        let started = Instant::now();
        cab.clone()
            .report_on(&ctx)
            .speed_kph(cab.speed_kph)
            .last_seen_at(UtcDateTimeUsec::now())
            .await
            .expect("reports");
        report.push(us(started.elapsed()));
    }
    let mut record = Vec::new();
    for cab in cabs.iter().cycle().take(samples) {
        let started = Instant::now();
        TelemetrySample::record(&ctx)
            .cab_id(cab.id)
            .recorded_at(UtcDateTimeUsec::now())
            .lng(cab.lng)
            .lat(cab.lat)
            .speed_kph(0)
            .battery_pct(cab.battery_pct)
            .await
            .expect("records");
        record.push(us(started.elapsed()));
    }

    // The simulation, counting every action it takes.
    let counter = Arc::new(Counter::default());
    let counted = ctx
        .clone()
        .with_notifier(counter.clone() as Arc<dyn Notifier>);
    let config = SimConfig {
        speedup: 8.0,
        demand: 1.0,
        seed: 7,
    };
    let mut sim = Simulation::new(counted, city, config)
        .await
        .expect("simulates");
    for _ in 0..warmup {
        sim.step().await.expect("steps");
    }
    let before = counter.0.load(Ordering::Relaxed);
    let mut tick = Vec::new();
    let started = Instant::now();
    for _ in 0..ticks {
        let at = Instant::now();
        sim.step().await.expect("steps");
        tick.push(ms(at.elapsed()));
    }
    let elapsed = started.elapsed();
    let actions = (counter.0.load(Ordering::Relaxed) - before) as f64;

    let cabs = Cab::query(&ctx).all().await.expect("reads");
    let count = |status: &str| cabs.iter().filter(|c| c.status == status).count() as i64;
    let in_service = (cabs.len() as i64 - count("maintenance")).max(1);

    FleetReport {
        fleet,
        seed_s: seed.as_secs_f64(),
        get_us: median(get),
        read_all_ms: median(read_all),
        find_by_call_sign_ms: median(find),
        report_us: median(report),
        record_us: median(record),
        tick_ms: Spread::of(tick),
        actions_per_tick: actions / ticks as f64,
        actions_per_s: actions / elapsed.as_secs_f64(),
        busy_pct: (count("dispatched") + count("on_trip")) * 100 / in_service,
    }
}

/// One subscriber's view of the run.
#[derive(Default)]
struct Watcher {
    received: AtomicUsize,
    ended: AtomicBool,
    latencies: Mutex<Vec<f64>>,
}

async fn watch(addr: String, watcher: Arc<Watcher>, ready: Arc<AtomicUsize>) {
    let mut request = format!("ws://{addr}/graphql/ws")
        .into_client_request()
        .expect("a request");
    request.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        "graphql-transport-ws".parse().expect("a header"),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .expect("connects");
    let send = |value: Value| Message::Text(value.to_string().into());
    socket
        .send(send(json!({ "type": "connection_init" })))
        .await
        .expect("sends");
    socket.next().await.expect("an ack").expect("an ack");
    socket
        .send(send(json!({
            "id": "cabs",
            "type": "subscribe",
            "payload": { "query": "subscription { cabUpdated { id lng lat heading_deg speed_kph battery_pct last_seen_at } }" },
        })))
        .await
        .expect("subscribes");
    ready.fetch_add(1, Ordering::SeqCst);
    while let Some(Ok(message)) = socket.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let message: Value = serde_json::from_str(&text).expect("json");
        match message["type"].as_str() {
            Some("next") => {
                let sent = message["payload"]["data"]["cabUpdated"]["last_seen_at"]
                    .as_str()
                    .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok());
                if let Some(sent) = sent {
                    let latency = chrono::Utc::now().signed_duration_since(sent);
                    watcher
                        .latencies
                        .lock()
                        .expect("a lock")
                        .push(latency.num_microseconds().unwrap_or(0) as f64 / 1e3);
                }
                watcher.received.fetch_add(1, Ordering::SeqCst);
            }
            Some("complete") | Some("error") => break,
            _ => {}
        }
    }
    watcher.ended.store(true, Ordering::SeqCst);
}

async fn fanout(fleet: usize, subscribers: usize, rounds: usize) -> FanoutReport {
    let pubsub = PubSub::new();
    let (ctx, _, _) = seeded(fleet, &pubsub).await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("binds");
    let addr = listener.local_addr().expect("an address").to_string();
    let app = router(ctx.clone(), pubsub).expect("a router");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serves") });

    let ready = Arc::new(AtomicUsize::new(0));
    let watchers: Vec<Arc<Watcher>> = (0..subscribers)
        .map(|_| Arc::new(Watcher::default()))
        .collect();
    let clients: Vec<_> = watchers
        .iter()
        .map(|w| tokio::spawn(watch(addr.clone(), w.clone(), ready.clone())))
        .collect();
    while ready.load(Ordering::SeqCst) < subscribers {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // The protocol doesn't acknowledge a subscription: report until everyone hears one.
    let cabs = Cab::query(&ctx).all().await.expect("reads");
    let started = Instant::now();
    while watchers
        .iter()
        .any(|w| w.received.load(Ordering::SeqCst) == 0)
        && started.elapsed() < Duration::from_secs(30)
    {
        cabs[0]
            .clone()
            .report_on(&ctx)
            .last_seen_at(UtcDateTimeUsec::now())
            .await
            .expect("reports");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    for w in &watchers {
        w.received.store(0, Ordering::SeqCst);
        w.latencies.lock().expect("a lock").clear();
    }

    let mut publish = Vec::new();
    let total = || {
        watchers
            .iter()
            .map(|w| w.received.load(Ordering::SeqCst))
            .sum::<usize>()
    };
    for round in 1..=rounds {
        let started = Instant::now();
        for cab in &cabs {
            cab.clone()
                .report_on(&ctx)
                .last_seen_at(UtcDateTimeUsec::now())
                .await
                .expect("reports");
        }
        publish.push(ms(started.elapsed()));
        // Wait for this round to land, or for deliveries to stop.
        let target = round * fleet * subscribers;
        let (mut last, mut idle) = (total(), Instant::now());
        while total() < target && idle.elapsed() < Duration::from_secs(2) {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let now = total();
            if now != last {
                (last, idle) = (now, Instant::now());
            }
        }
    }

    let delivered = total();
    let ended = watchers
        .iter()
        .filter(|w| w.ended.load(Ordering::SeqCst))
        .count();
    let latencies = watchers
        .iter()
        .flat_map(|w| w.latencies.lock().expect("a lock").clone())
        .collect();
    for client in clients {
        client.abort();
    }
    server.abort();
    FanoutReport {
        fleet,
        subscribers,
        publish_round_ms: Spread::of(publish),
        expected: rounds * fleet * subscribers,
        delivered,
        latency_ms: Spread::of(latencies),
        ended,
    }
}

fn print_fleet(r: &FleetReport) {
    println!(
        "| {:>6} | {:>7.1} | {:>6.1} | {:>8.2} | {:>8.2} | {:>7.1} | {:>7.1} | {:>8.1} | {:>8.1} | {:>8.1} | {:>7.0} | {:>8.0} | {:>4}% |",
        r.fleet,
        r.seed_s,
        r.get_us,
        r.read_all_ms,
        r.find_by_call_sign_ms,
        r.report_us,
        r.record_us,
        r.tick_ms.p50,
        r.tick_ms.p95,
        r.tick_ms.max,
        r.actions_per_tick,
        r.actions_per_s,
        r.busy_pct,
    );
}

fn print_fanout(r: &FanoutReport) {
    println!(
        "| {:>6} | {:>4} | {:>8.1} | {:>9} | {:>9} | {:>5.1}% | {:>8.1} | {:>8.1} | {:>8.1} | {:>5} |",
        r.fleet,
        r.subscribers,
        r.publish_round_ms.p50,
        r.expected,
        r.delivered,
        r.delivered as f64 * 100.0 / r.expected.max(1) as f64,
        r.latency_ms.p50,
        r.latency_ms.p99,
        r.latency_ms.max,
        r.ended,
    );
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args = args();
    let mut report = Report {
        machine: format!(
            "{} {}, {} threads",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism().map_or(0, |n| n.get())
        ),
        ..Report::default()
    };
    println!("\nCybercab scale · {}\n", report.machine);

    println!(
        "Simulation: one tick is one second of a live fleet, so it must stay under 1000 ms.\n"
    );
    println!(
        "|  fleet | seed s | get µs | all cabs ms | by call sign ms | report µs | record µs | tick p50 ms | tick p95 ms | tick max ms | actions/tick | actions/s | busy |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    let mut previous: Option<(usize, f64)> = None;
    for &size in &args.fleets {
        // Skip sizes projected to blow the budget, assuming the run grows with the square
        // of the fleet: every cab's work touching every other cab.
        if let Some((fleet, run_s)) = previous {
            let ratio = size as f64 / fleet as f64;
            let projected = run_s * ratio * ratio;
            if projected > args.budget.as_secs_f64() {
                println!(
                    "| {size:>6} | skipped: projected at {projected:.0} s, over the {} s budget |",
                    args.budget.as_secs()
                );
                continue;
            }
        }
        let started = Instant::now();
        let r = fleet(size, args.warmup, args.ticks).await;
        print_fleet(&r);
        previous = Some((size, started.elapsed().as_secs_f64()));
        report.fleets.push(r);
    }

    println!("\nFan-out: every cab reports once per round, over /graphql/ws to each subscriber.\n");
    println!(
        "|  fleet | subs | publish round ms | expected | delivered | delivered | latency p50 ms | p99 ms | max ms | ended |"
    );
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for &size in &args.fanout_fleets {
        for &subs in &args.subscribers {
            let r = fanout(size, subs, args.rounds).await;
            print_fanout(&r);
            report.fanout.push(r);
        }
    }

    if let Some(path) = args.json {
        std::fs::write(&path, serde_json::to_string_pretty(&report).expect("json"))
            .expect("writes the report");
        println!("\nWrote {path}");
    }
}
