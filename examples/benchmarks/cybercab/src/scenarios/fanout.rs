//! D: the fleet's movements, heard by a rising number of subscribers: how many events
//! each hears, how late, whether any subscription ends, and what the server spends.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};

use super::{Plan, ticks_between};
use crate::gql::{Heard, age_ms, subscribe};
use crate::record::Record;
use crate::server::Running;
use crate::stats::Summary;

/// The cab's position reports. A heartbeat written as an upsert notifies as a create, so
/// subscribers listen for both.
pub const CAB_MOVES: [&str; 2] = [
    "subscription { cabUpdated { updated { id lastSeenAt } } }",
    "subscription { cabCreated { created { id lastSeenAt } } }",
];

/// What a group of subscribers heard over a window.
#[derive(Default)]
pub struct Listened {
    pub events: Vec<usize>,
    pub delivery_ms: Vec<f64>,
    pub ended: usize,
    pub failed_to_connect: usize,
}

/// Subscribes `count` connections to the cabs' movements from now until `until`,
/// counting what they hear from `from`.
pub async fn listen(base: &str, count: usize, from: Instant, until: Instant) -> Listened {
    let heard = Arc::new(Mutex::new(Listened::default()));
    let mut tasks = Vec::new();
    for _ in 0..count {
        let heard = Arc::clone(&heard);
        let base = base.to_string();
        tasks.push(tokio::spawn(async move {
            let mut events = 0usize;
            let mut delivery = Vec::new();
            let mut ended = 0;
            // The last report heard from each cab: only a new report says how late it is.
            let mut last_report: HashMap<String, String> = HashMap::new();
            let result = subscribe(&base, &CAB_MOVES, until, |event| match event {
                Heard::Next(data) => {
                    let cab = data
                        .get("cabUpdated")
                        .map(|d| &d["updated"])
                        .or_else(|| data.get("cabCreated").map(|d| &d["created"]))
                        .cloned()
                        .unwrap_or(Value::Null);
                    let (Some(id), Some(seen)) = (cab["id"].as_str(), cab["lastSeenAt"].as_str()) else {
                        return;
                    };
                    let previous = last_report.insert(id.to_string(), seen.to_string());
                    if Instant::now() < from {
                        return;
                    }
                    events += 1;
                    if previous.is_some_and(|p| p != seen)
                        && let Some(age) = age_ms(seen)
                    {
                        delivery.push(age);
                    }
                }
                Heard::Ended => {
                    if Instant::now() >= from {
                        ended += 1;
                    }
                }
            })
            .await;
            let mut all = heard.lock().expect("heard");
            if result.is_err() {
                all.failed_to_connect += 1;
            }
            all.events.push(events);
            all.delivery_ms.extend(delivery);
            all.ended += ended;
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
    Arc::try_unwrap(heard).ok().expect("every listener done").into_inner().expect("heard")
}

pub async fn run(plan: &Plan, server: &Running, fleet: usize, rep: usize) -> Vec<Record> {
    let mut records = Vec::new();
    for &subscribers in &plan.subscribers {
        // Time to connect and subscribe, then the warm-up.
        let from = Instant::now() + plan.warmup + std::time::Duration::from_secs(1);
        let until = from + plan.fanout_measure;
        let before = server.metrics().await;
        let watcher = server.watch();
        let mut heard = listen(server.api.base(), subscribers, from, until).await;
        let resources = watcher.finish();
        let after = server.metrics().await;
        let seconds = plan.fanout_measure.as_secs_f64();
        heard.events.sort_unstable();
        let per_subscriber = heard.events.get(heard.events.len() / 2).copied().unwrap_or(0) as f64 / seconds;
        records.push(Record {
            scenario: "fanout".into(),
            server: server.label.clone(),
            params: json!({ "subscribers": subscribers, "fleet": fleet }),
            metrics: json!({
                "events_per_s_per_subscriber": per_subscriber,
                "events_per_s_total": heard.events.iter().sum::<usize>() as f64 / seconds,
                "delivery_ms": Summary::of_ms(&mut heard.delivery_ms),
                "ended": heard.ended,
                "failed_to_connect": heard.failed_to_connect,
                "ticks": ticks_between(&before, &after),
                "resources": resources,
            }),
            rep,
        });
    }
    records
}
