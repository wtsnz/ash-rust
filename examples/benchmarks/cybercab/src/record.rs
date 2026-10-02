//! What a run measured, one JSON line per scenario, server and setting.

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub scenario: String,
    pub server: String,
    /// What varied: fleet size, concurrency, subscribers, query.
    pub params: Value,
    pub metrics: Value,
    pub rep: usize,
}

pub fn append(out: &Path, record: &Record) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("results.jsonl"))
        .expect("the results file");
    writeln!(file, "{}", serde_json::to_string(record).expect("json")).expect("a write");
    println!(
        "  {:<9} {:<18} {} {}",
        record.scenario, record.server, record.params, record.metrics
    );
}

pub fn read(out: &Path) -> Vec<Record> {
    std::fs::read_to_string(out.join("results.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}
