//! The scenarios, each run against every server the same way.

pub mod commands;
pub mod fanout;
pub mod load;
pub mod reads;
pub mod room;
pub mod sim;

use std::time::Duration;

use serde_json::Value;

use crate::server::Running;
use crate::stats::Summary;

/// How long and how hard each scenario runs. Every scenario runs against one server per
/// fleet size, booted once with the simulation going, so a run spends its time measuring
/// rather than seeding.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Fleets to run at: a server is booted, and every scenario run, at each.
    pub fleets: Vec<usize>,
    /// After boot, before measuring: the simulation settles into its stride.
    pub settle: Duration,
    /// Before each measurement, not counted.
    pub warmup: Duration,
    /// Each read or command, at each concurrency level.
    pub measure: Duration,
    /// Concurrent clients, for reads and commands.
    pub levels: Vec<usize>,
    /// The simulation on its own.
    pub sim_measure: Duration,
    /// Subscribers to the fleet's movements, a measurement each.
    pub subscribers: Vec<usize>,
    pub fanout_measure: Duration,
    pub room_operators: usize,
    pub room_displays: usize,
    pub room_measure: Duration,
    /// Only these reads, if any.
    pub queries: Vec<String>,
}

impl Plan {
    /// About two minutes: one fleet, short windows.
    pub fn standard() -> Self {
        Self {
            fleets: vec![200],
            settle: Duration::from_secs(2),
            warmup: Duration::from_millis(300),
            measure: Duration::from_secs(1),
            levels: vec![1, 16],
            sim_measure: Duration::from_secs(4),
            subscribers: vec![10],
            fanout_measure: Duration::from_secs(4),
            room_operators: 8,
            room_displays: 20,
            room_measure: Duration::from_secs(4),
            queries: Vec::new(),
        }
    }

    /// Bigger fleets, longer windows, more levels: closer to the truth, and much slower.
    pub fn full() -> Self {
        Self {
            fleets: vec![200, 1000, 5000],
            settle: Duration::from_secs(10),
            warmup: Duration::from_secs(2),
            measure: Duration::from_secs(8),
            levels: vec![1, 16, 64],
            sim_measure: Duration::from_secs(30),
            subscribers: vec![1, 10, 100],
            fanout_measure: Duration::from_secs(15),
            room_operators: 8,
            room_displays: 20,
            room_measure: Duration::from_secs(30),
            queries: Vec::new(),
        }
    }
}

/// The simulation's ticks between two reads of `/metrics`: how many ran, how long they
/// took (the most recent 120), and how many failed.
pub fn ticks_between(before: &Value, after: &Value) -> Value {
    let ran = after["ticks"].as_u64().unwrap_or(0).saturating_sub(before["ticks"].as_u64().unwrap_or(0));
    let errors = after["errors"].as_u64().unwrap_or(0).saturating_sub(before["errors"].as_u64().unwrap_or(0));
    let recent: Vec<f64> = after["recent_tick_ms"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_f64).collect())
        .unwrap_or_default();
    let mut window: Vec<f64> = recent[recent.len().saturating_sub(ran as usize)..].to_vec();
    let over_budget = window.iter().filter(|ms| **ms > 1000.0).count();
    serde_json::json!({
        "ticks": ran,
        "errors": errors,
        "over_budget": over_budget,
        "tick_ms": Summary::of_ms(&mut window),
    })
}

/// Waits `duration`, reading `/metrics` before and after.
pub async fn tick_window(server: &Running, duration: Duration) -> Value {
    let before = server.metrics().await;
    tokio::time::sleep(duration).await;
    let after = server.metrics().await;
    let mut ticks = ticks_between(&before, &after);
    ticks["ticks_per_s"] = (ticks["ticks"].as_f64().unwrap_or(0.0) / duration.as_secs_f64()).into();
    ticks
}
