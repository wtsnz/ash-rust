//! The servers under test: started fresh for every run with the settings it needs, timed
//! from start to healthy, and watched for CPU and memory while it runs.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::process::{Child, Command};

use crate::gql::Graphql;

/// Which server, and how it writes the fleet's heartbeat.
#[derive(Clone, Debug)]
pub enum Kind {
    /// The Rust Cybercab: `bulk_update` of every cab's report.
    Rust { bin: PathBuf },
    /// The Elixir Cybercab's release, with `HEARTBEAT` set to `heartbeat`.
    Elixir { release: PathBuf, heartbeat: &'static str },
}

impl Kind {
    /// `rust`, `elixir-concurrent` or `elixir-upsert`; `elixir` when the simulation is off,
    /// where the heartbeat doesn't run.
    pub fn label(&self, simulating: bool) -> String {
        match self {
            Kind::Rust { .. } => "rust".into(),
            Kind::Elixir { .. } if !simulating => "elixir".into(),
            Kind::Elixir { heartbeat, .. } => format!("elixir-{heartbeat}"),
        }
    }

    fn port(&self) -> u16 {
        match self {
            Kind::Rust { .. } => 4620,
            Kind::Elixir { .. } => 4630,
        }
    }

    fn database(&self) -> &'static str {
        match self {
            Kind::Rust { .. } => "cybercab_rust",
            Kind::Elixir { .. } => "cybercab_elixir",
        }
    }
}

/// How to start a server for a run.
#[derive(Clone, Debug)]
pub struct Launch {
    pub kind: Kind,
    pub fleet: usize,
    pub simulating: bool,
    /// `postgres://user:pass@host:port`, without a database.
    pub postgres: String,
}

/// A server, running.
pub struct Running {
    child: Child,
    pub label: String,
    pub api: Graphql,
    /// From spawning it to `/health` answering: migrating, emptying and seeding included.
    pub boot: Duration,
    /// The process doing the work: the one listening on the port.
    pub pid: u32,
}

impl Launch {
    pub async fn start(&self) -> Running {
        let port = self.kind.port();
        wait_until_free(port).await;
        let mut command = match &self.kind {
            Kind::Rust { bin } => Command::new(bin),
            Kind::Elixir { release, heartbeat } => {
                let mut command = Command::new(release.join("bin/cybercab"));
                // No distribution: one node, no epmd.
                command
                    .arg("start")
                    .env("HEARTBEAT", heartbeat)
                    .env("RELEASE_DISTRIBUTION", "none");
                command
            }
        };
        command
            .env("DATABASE_URL", format!("{}/{}", self.postgres, self.kind.database()))
            .env("PORT", port.to_string())
            .env("FLEET", self.fleet.to_string())
            .env("SIM", if self.simulating { "on" } else { "off" })
            .env("SIM_SPEED", "8")
            .env("DEMAND", "1")
            .env("SEED", "51893")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let started = Instant::now();
        let child = command.spawn().expect("the server starts");
        let api = Graphql::new(&format!("http://127.0.0.1:{port}"));
        let deadline = started + Duration::from_secs(900);
        while !api.healthy().await {
            assert!(Instant::now() < deadline, "{} never became healthy", self.kind.label(self.simulating));
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let boot = started.elapsed();
        let pid = listening_pid(port).await.unwrap_or_else(|| child.id().expect("a pid"));
        Running {
            child,
            label: self.kind.label(self.simulating),
            api,
            boot,
            pid,
        }
    }
}

impl Running {
    pub async fn stop(mut self) {
        let _ = Command::new("kill").arg(self.pid.to_string()).status().await;
        if let Some(id) = self.child.id() {
            let _ = Command::new("kill").arg(id.to_string()).status().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(20), self.child.wait()).await;
        let _ = self.child.kill().await;
    }

    /// Starts sampling the server's CPU and memory every half second.
    pub fn watch(&self) -> Watcher {
        Watcher::start(self.pid)
    }

    /// `/metrics`: the simulation's ticks.
    pub async fn metrics(&self) -> serde_json::Value {
        self.api.get_json("/metrics").await.unwrap_or_default()
    }
}

async fn listening_pid(port: u16) -> Option<u32> {
    let output = Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .await
        .ok()?;
    String::from_utf8_lossy(&output.stdout).lines().next()?.trim().parse().ok()
}

async fn wait_until_free(port: u16) {
    for _ in 0..200 {
        if listening_pid(port).await.is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("port {port} stayed in use");
}

/// CPU and memory a server used over a stretch of a run.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize)]
pub struct Resources {
    /// CPU time over wall time: 1.0 is one core busy throughout.
    pub cpu_cores: f64,
    pub peak_rss_mb: f64,
    pub mean_rss_mb: f64,
}

/// Samples a process's CPU time and resident memory with `ps`.
pub struct Watcher {
    samples: Arc<Mutex<Vec<(Instant, f64, f64)>>>,
    task: tokio::task::JoinHandle<()>,
}

impl Watcher {
    fn start(pid: u32) -> Self {
        let samples = Arc::new(Mutex::new(Vec::new()));
        let shared = Arc::clone(&samples);
        let task = tokio::spawn(async move {
            loop {
                if let Some((cpu_s, rss_mb)) = sample(pid).await {
                    shared.lock().expect("samples").push((Instant::now(), cpu_s, rss_mb));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        });
        Self { samples, task }
    }

    pub fn finish(self) -> Resources {
        self.task.abort();
        let samples = self.samples.lock().expect("samples");
        let (Some(first), Some(last)) = (samples.first(), samples.last()) else {
            return Resources::default();
        };
        let wall = last.0.duration_since(first.0).as_secs_f64();
        let rss: Vec<f64> = samples.iter().map(|s| s.2).collect();
        Resources {
            cpu_cores: if wall > 0.0 { (last.1 - first.1) / wall } else { 0.0 },
            peak_rss_mb: rss.iter().copied().fold(0.0, f64::max),
            mean_rss_mb: rss.iter().sum::<f64>() / rss.len() as f64,
        }
    }
}

/// `(cpu seconds so far, resident MB)` for `pid`.
async fn sample(pid: u32) -> Option<(f64, f64)> {
    let output = Command::new("ps")
        .args(["-o", "time=,rss=", "-p", &pid.to_string()])
        .output()
        .await
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut fields = text.split_whitespace();
    let time = fields.next()?;
    let rss_kb: f64 = fields.next()?.parse().ok()?;
    Some((cpu_seconds(time)?, rss_kb / 1024.0))
}

/// `ps`'s `time`: `[[dd-]hh:]mm:ss.cc`.
fn cpu_seconds(time: &str) -> Option<f64> {
    let (days, rest) = match time.split_once('-') {
        Some((days, rest)) => (days.parse::<f64>().ok()?, rest),
        None => (0.0, time),
    };
    let parts: Vec<f64> = rest.split(':').map(|p| p.parse().ok()).collect::<Option<_>>()?;
    let seconds = parts.iter().fold(0.0, |total, part| total * 60.0 + part);
    Some(days * 86_400.0 + seconds)
}

#[cfg(test)]
mod tests {
    #[test]
    fn reads_ps_cpu_time() {
        assert_eq!(super::cpu_seconds("0:01.50"), Some(1.5));
        assert_eq!(super::cpu_seconds("1:02:03.00"), Some(3723.0));
        assert_eq!(super::cpu_seconds("2-00:00:01.00"), Some(172_801.0));
    }
}
