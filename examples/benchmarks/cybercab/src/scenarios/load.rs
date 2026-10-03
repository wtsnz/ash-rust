//! Closed-loop load: `workers` clients each sending a request as soon as the last came
//! back, for a while, after a warm-up that isn't counted.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::gql::Response;
use crate::stats::{Latencies, Summary};

#[derive(Clone, Debug, serde::Serialize)]
pub struct LoadResult {
    pub requests_per_s: f64,
    pub latency_ms: Summary,
    pub failed: usize,
}

/// Runs `request(worker, n)` from `workers` concurrent loops: `warmup`, then `measure`.
pub async fn run<F, Fut>(workers: usize, warmup: Duration, measure: Duration, request: F) -> LoadResult
where
    F: Fn(usize, u64) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Response> + Send,
{
    let request = Arc::new(request);
    let start = Instant::now();
    let counted_from = start + warmup;
    let until = counted_from + measure;
    let mut tasks = Vec::new();
    for worker in 0..workers {
        let request = Arc::clone(&request);
        tasks.push(tokio::spawn(async move {
            let mut latencies = Latencies::default();
            let mut failed = 0;
            let mut n = 0u64;
            while Instant::now() < until {
                let response = request(worker, n).await;
                n += 1;
                if Instant::now() >= counted_from {
                    if response.failed {
                        failed += 1;
                    } else {
                        latencies.record(response.elapsed);
                    }
                }
            }
            (latencies, failed)
        }));
    }
    let mut latencies = Latencies::default();
    let mut failed = 0;
    for task in tasks {
        let (l, f) = task.await.expect("a worker");
        latencies.extend(l);
        failed += f;
    }
    LoadResult {
        requests_per_s: latencies.len() as f64 / measure.as_secs_f64(),
        latency_ms: latencies.summary(),
        failed,
    }
}
