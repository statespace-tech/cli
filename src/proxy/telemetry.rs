//! Events proxies and controllers send to Statespace: runs, and each arm's
//! requests, output, metrics, and lifecycle events. The default is never
//! recorded. Sending never blocks or slows the app: when the buffer is
//! full, events are dropped and counted.

use std::{
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use super::{
    assign::{Assignment, Key},
    remote::Remote,
};

/// Events waiting to be sent, at most.
const BUFFER: usize = 100_000;
/// Events per request to the API, at most.
const BATCH: usize = 1000;
/// How long a proxy remembers that it recorded a subject's run.
const RUN_MEMORY: Duration = Duration::from_secs(30 * 60);

/// A run a proxy recorded: its experiment, version, subject, and reason.
type RunKey = (String, u32, String, &'static str);

pub struct Telemetry {
    host: String,
    sender: Option<mpsc::Sender<(&'static str, Value)>>,
    dropped: AtomicU64,
    runs: Mutex<HashMap<RunKey, Instant>>,
    http: Mutex<HashMap<Key, Window>>,
}

/// One minute of an arm's HTTP traffic.
#[derive(Default)]
struct Window {
    durations: Vec<f64>,
    statuses: HashMap<String, u64>,
    errors: u64,
}

impl Telemetry {
    /// Telemetry that sends to `remote`, or discards events without it.
    pub fn start(host: String, remote: Option<Remote>) -> Self {
        let sender = remote.map(|remote| {
            let (sender, receiver) = mpsc::channel(BUFFER);
            tokio::spawn(deliver(remote, receiver));
            sender
        });
        Self {
            host,
            sender,
            dropped: AtomicU64::new(0),
            runs: Mutex::default(),
            http: Mutex::default(),
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn send(&self, table: &'static str, mut event: Value) {
        let Some(sender) = &self.sender else {
            return;
        };
        event["id"] = json!(id(table));
        event["host"] = json!(self.host);
        event["timestamp"] = json!(now());
        if sender.try_send((table, event)).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a subject's assignment the first time this proxy routes it.
    pub fn run(&self, assignment: &Assignment, method: &str, path: &str) {
        let key = (
            assignment.experiment.clone(),
            assignment.version,
            assignment.subject.clone(),
            assignment.reason.as_str(),
        );
        {
            let mut runs = self.runs.lock().expect("run memory is never poisoned");
            let now = Instant::now();
            if runs
                .get(&key)
                .is_some_and(|seen| now.duration_since(*seen) < RUN_MEMORY)
            {
                return;
            }
            if runs.len() > 200_000 {
                runs.retain(|_, seen| now.duration_since(*seen) < RUN_MEMORY);
            }
            runs.insert(key, now);
        }
        self.send(
            "runs",
            json!({
                "experiment": assignment.experiment,
                "version": assignment.version,
                "subject_id": assignment.subject,
                "variant": assignment.assigned.as_ref().map(|key| &key.arm),
                "variant_version": assignment.assigned.as_ref().and_then(|key| key.version),
                "reason": assignment.reason.as_str(),
                "context": {"method": method, "path": path},
            }),
        );
    }

    /// An arm's fields every event about it carries.
    fn arm(key: &Key, mut event: Value) -> Value {
        event["experiment"] = json!(key.experiment);
        event["experiment_version"] = json!(key.experiment_version);
        event["variant"] = json!(key.arm);
        event["variant_version"] = json!(key.version);
        event
    }

    pub fn output(&self, key: &Key, stream: &str, line: String) {
        self.send(
            "output",
            Self::arm(key, json!({"stream": stream, "line": line})),
        );
    }

    pub fn lifecycle(&self, key: &Key, event: &str, data: Value) {
        self.send(
            "lifecycle",
            Self::arm(key, json!({"event": event, "data": data})),
        );
    }

    pub fn resources(&self, key: &Key, data: Value) {
        self.send(
            "metrics",
            Self::arm(key, json!({"kind": "resources", "data": data})),
        );
    }

    pub fn request(&self, request: Value) {
        self.send("requests", request);
    }

    /// Count one finished request toward its arm's per-minute summary.
    pub fn record_http(&self, key: &Key, status: u16, duration_ms: f64) {
        let mut windows = self.http.lock().expect("HTTP windows are never poisoned");
        let window = windows.entry(key.clone()).or_default();
        window.durations.push(duration_ms);
        *window
            .statuses
            .entry(format!("{}xx", status / 100))
            .or_default() += 1;
        if status >= 500 {
            window.errors += 1;
        }
    }

    /// Send and reset the per-minute HTTP summaries.
    pub fn flush_http(&self) {
        let windows =
            std::mem::take(&mut *self.http.lock().expect("HTTP windows are never poisoned"));
        for (key, mut window) in windows {
            window.durations.sort_by(f64::total_cmp);
            let percentile = |share: f64| {
                let index = ((window.durations.len() as f64 - 1.0) * share).round() as usize;
                window.durations.get(index).copied()
            };
            let statuses: Map<String, Value> = window
                .statuses
                .into_iter()
                .map(|(class, count)| (class, json!(count)))
                .collect();
            self.send(
                "metrics",
                Self::arm(
                    &key,
                    json!({
                        "kind": "http",
                        "data": {
                            "requests": window.durations.len(),
                            "errors": window.errors,
                            "statuses": statuses,
                            "p50_ms": percentile(0.5),
                            "p90_ms": percentile(0.9),
                            "p99_ms": percentile(0.99),
                            "max_ms": window.durations.last(),
                        },
                    }),
                ),
            );
        }
    }
}

/// Send events in batches. A batch that keeps failing is dropped and counted.
async fn deliver(remote: Remote, mut receiver: mpsc::Receiver<(&'static str, Value)>) {
    let mut pending: Vec<(&'static str, Value)> = Vec::new();
    loop {
        let deadline = tokio::time::sleep(Duration::from_secs(1));
        tokio::pin!(deadline);
        while pending.len() < BATCH {
            tokio::select! {
                received = receiver.recv() => match received {
                    Some(event) => pending.push(event),
                    None if pending.is_empty() => return,
                    None => break,
                },
                _ = &mut deadline => break,
            }
        }
        if pending.is_empty() {
            continue;
        }
        let mut batch = Map::new();
        for (table, event) in pending.drain(..) {
            batch
                .entry(table)
                .or_insert_with(|| Value::Array(Vec::new()))
                .as_array_mut()
                .expect("every table holds an array")
                .push(event);
        }
        let mut delay = Duration::from_millis(500);
        for attempt in 1..=5 {
            match remote.send_events(Value::Object(batch.clone())).await {
                Ok(()) => break,
                Err(error) if attempt == 5 => {
                    eprintln!("[ssp] dropped {} events: {error:#}", count(&batch));
                }
                Err(_) => {
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
            }
        }
    }
}

fn count(batch: &Map<String, Value>) -> usize {
    batch
        .values()
        .filter_map(Value::as_array)
        .map(Vec::len)
        .sum()
}

pub fn id(prefix: &str) -> String {
    format!("{prefix}_{:032x}", rand::random::<u128>())
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::assign::Reason;

    #[tokio::test]
    async fn records_each_runs_first_exposure_once() {
        let (sender, mut receiver) = mpsc::channel(10);
        let telemetry = Telemetry {
            host: "host-1".into(),
            sender: Some(sender),
            dropped: AtomicU64::new(0),
            runs: Mutex::default(),
            http: Mutex::default(),
        };
        let control = Key {
            experiment: "checkout".into(),
            experiment_version: 1,
            arm: "control".into(),
            version: None,
        };
        let assignment = Assignment {
            experiment: "checkout".into(),
            version: 1,
            subject: "u_1".into(),
            assigned: Some(control.clone()),
            reason: Reason::Assigned,
        };
        telemetry.run(&assignment, "GET", "/");
        telemetry.run(&assignment, "GET", "/again");
        telemetry.record_http(&control, 200, 10.0);
        telemetry.record_http(&control, 503, 30.0);
        telemetry.flush_http();
        let (table, run) = receiver.recv().await.unwrap();
        assert_eq!((table, run["reason"].as_str()), ("runs", Some("assigned")));
        assert_eq!(run["variant"], "control");
        let (table, metric) = receiver.recv().await.unwrap();
        assert_eq!(table, "metrics");
        assert_eq!(metric["data"]["requests"], 2);
        assert_eq!(
            (&metric["experiment"], &metric["variant"]),
            (&json!("checkout"), &json!("control"))
        );
        assert_eq!(metric["data"]["errors"], 1);
        assert_eq!(metric["data"]["p50_ms"], 30.0);
        assert!(
            receiver.try_recv().is_err(),
            "the second exposure is not recorded"
        );
    }
}
