//! E: the real room. The fleet moves, wall displays follow it, and operators read and
//! command at once: how fast the operators are served, whether the simulation keeps its
//! ticks, and how late the displays hear things.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Plan, fanout, ticks_between};
use crate::record::Record;
use crate::server::Running;
use crate::stats::{Latencies, Summary};

const OPERATOR_LOOP: [&str; 6] = ["fleet", "pullOverCab", "trips_with_riders", "resumeCab", "cab_by_id", "aggregates"];
/// An operator's pause between actions.
const THINK: Duration = Duration::from_millis(250);

pub async fn run(plan: &Plan, server: &Running, fleet: usize, rep: usize) -> Record {
    let api = server.api.clone();
    let cabs: Vec<String> = api.must("{ listCabs { results { id } } }", json!({})).await["listCabs"]["results"]
        .as_array()
        .expect("cabs")
        .iter()
        .map(|c| c["id"].as_str().expect("an id").to_string())
        .collect();
    let cabs = Arc::new(cabs);

    let from = Instant::now() + plan.warmup + Duration::from_secs(1);
    let until = from + plan.room_measure;
    let before = server.metrics().await;
    let watcher = server.watch();
    let displays = tokio::spawn({
        let base = api.base().to_string();
        let count = plan.room_displays;
        async move { fanout::listen(&base, count, from, until).await }
    });

    let mut operators = Vec::new();
    for operator in 0..plan.room_operators {
        let (api, cabs) = (api.clone(), Arc::clone(&cabs));
        operators.push(tokio::spawn(async move {
            let (mut reads, mut commands, mut failed) = (Latencies::default(), Latencies::default(), 0usize);
            let mut n = 0usize;
            while Instant::now() < until {
                let cab = &cabs[(operator * 131 + n / OPERATOR_LOOP.len()) % cabs.len()];
                let action = OPERATOR_LOOP[n % OPERATOR_LOOP.len()];
                let response = match action {
                    "fleet" => api.request(super::reads::FLEET_QUERY, json!({})).await,
                    "trips_with_riders" => api.request(super::reads::TRIPS_QUERY, json!({})).await,
                    "cab_by_id" => api.request(super::reads::CAB_QUERY, json!({ "id": cab })).await,
                    "aggregates" => api.request(super::reads::AGGREGATES_QUERY, json!({})).await,
                    command => {
                        let mutation = format!(
                            "mutation($id: ID!) {{ {command}(id: $id) {{ result {{ halted }} errors {{ message }} }} }}"
                        );
                        api.request(&mutation, json!({ "id": cab })).await
                    }
                };
                if Instant::now() >= from {
                    if response.failed {
                        failed += 1;
                    } else if action.ends_with("Cab") {
                        commands.record(response.elapsed);
                    } else {
                        reads.record(response.elapsed);
                    }
                }
                n += 1;
                tokio::time::sleep(THINK).await;
            }
            (reads, commands, failed)
        }));
    }
    let (mut reads, mut commands, mut failed) = (Latencies::default(), Latencies::default(), 0);
    for operator in operators {
        let (r, c, f) = operator.await.expect("an operator");
        reads.extend(r);
        commands.extend(c);
        failed += f;
    }
    let mut heard = displays.await.expect("the displays");
    let resources = watcher.finish();
    let after = server.metrics().await;
    let seconds = plan.room_measure.as_secs_f64();
    heard.events.sort_unstable();
    Record {
        scenario: "room".into(),
        server: server.label.clone(),
        params: json!({ "fleet": fleet, "operators": plan.room_operators, "displays": plan.room_displays }),
        metrics: json!({
            "read_ms": reads.summary(),
            "command_ms": commands.summary(),
            "operator_failures": failed,
            "delivery_ms": Summary::of_ms(&mut heard.delivery_ms),
            "events_per_s_per_display": heard.events.get(heard.events.len() / 2).copied().unwrap_or(0) as f64 / seconds,
            "ended": heard.ended,
            "ticks": ticks_between(&before, &after),
            "resources": resources,
        }),
        rep,
    }
}
