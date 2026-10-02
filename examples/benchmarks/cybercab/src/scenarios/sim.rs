//! C: the simulation alone, at a fleet size: how long its ticks take, whether they keep
//! to their one-second budget, and what it costs.

use serde_json::json;

use super::{Plan, tick_window};
use crate::record::Record;
use crate::server::Running;

pub async fn run(plan: &Plan, server: &Running, fleet: usize, rep: usize) -> Record {
    let watcher = server.watch();
    let ticks = tick_window(server, plan.sim_measure).await;
    let resources = watcher.finish();
    Record {
        scenario: "sim".into(),
        server: server.label.clone(),
        params: json!({ "fleet": fleet }),
        metrics: json!({ "ticks": ticks, "resources": resources }),
        rep,
    }
}
