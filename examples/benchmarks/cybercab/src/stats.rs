//! Latencies, and the percentiles the report shows of them.

use serde::Serialize;
use std::time::Duration;

/// Every latency of one kind of request in a run, in microseconds.
#[derive(Clone, Default)]
pub struct Latencies(Vec<u64>);

impl Latencies {
    pub fn record(&mut self, elapsed: Duration) {
        self.0.push(elapsed.as_micros() as u64);
    }

    pub fn extend(&mut self, other: Latencies) {
        self.0.extend(other.0);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn summary(&mut self) -> Summary {
        Summary::of_micros(&mut self.0)
    }
}

/// Percentiles of a set of measurements, in milliseconds.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Summary {
    pub count: usize,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub max: f64,
    pub mean: f64,
}

impl Summary {
    pub fn of_micros(values: &mut [u64]) -> Self {
        let mut ms: Vec<f64> = values.iter().map(|v| *v as f64 / 1000.0).collect();
        Self::of_ms(&mut ms)
    }

    pub fn of_ms(values: &mut [f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        values.sort_by(f64::total_cmp);
        let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
        Self {
            count: values.len(),
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: values[values.len() - 1],
            mean: values.iter().sum::<f64>() / values.len() as f64,
        }
    }
}
