//! Every line an arm's pods print, read through the Kubernetes API and
//! recorded as the arm's output. Each pod's position survives a restart of
//! the controller, so lines are recorded once.

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use serde_json::Value;
use tokio::task::JoinHandle;

use super::{kube::Kube, spec};
use crate::proxy::{assign::Key, telemetry::Telemetry};

/// The time of the last line recorded, by pod.
pub type Positions = Arc<Mutex<HashMap<String, String>>>;

pub struct Followers {
    kube: Kube,
    telemetry: Arc<Telemetry>,
    redact: Arc<Vec<Regex>>,
    positions: Positions,
    tasks: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl Followers {
    pub fn new(
        kube: Kube,
        telemetry: Arc<Telemetry>,
        redact: Vec<Regex>,
        positions: Positions,
    ) -> Self {
        Self {
            kube,
            telemetry,
            redact: Arc::new(redact),
            positions,
            tasks: Mutex::default(),
        }
    }

    /// Follow every running arm pod in `pods`, and stop following the rest.
    pub fn follow(&self, pods: &[Value], container: &str) {
        let mut tasks = self.tasks.lock().expect("followers are never poisoned");
        let mut live = HashSet::new();
        for pod in pods {
            let (Some(name), Some(key)) = (pod["metadata"]["name"].as_str(), spec::key_of(pod))
            else {
                continue;
            };
            if pod["status"]["phase"] != "Running" {
                continue;
            }
            live.insert(name.to_owned());
            if tasks.get(name).is_some_and(|task| !task.is_finished()) {
                continue;
            }
            tasks.insert(
                name.to_owned(),
                tokio::spawn(follow(
                    self.kube.clone(),
                    self.telemetry.clone(),
                    self.redact.clone(),
                    self.positions.clone(),
                    name.to_owned(),
                    container.to_owned(),
                    key,
                )),
            );
        }
        tasks.retain(|name, task| {
            let keep = live.contains(name);
            if !keep {
                task.abort();
            }
            keep
        });
    }

    /// The pods being followed.
    pub fn pods(&self) -> HashSet<String> {
        self.tasks
            .lock()
            .expect("followers are never poisoned")
            .keys()
            .cloned()
            .collect()
    }
}

/// Record one pod's lines until it is gone. A stream that ends, as when the
/// container restarts, is opened again from the last line recorded.
async fn follow(
    kube: Kube,
    telemetry: Arc<Telemetry>,
    redact: Arc<Vec<Regex>>,
    positions: Positions,
    pod: String,
    container: String,
    key: Key,
) {
    let mut failures = 0;
    while failures < 5 {
        let last: Option<DateTime<Utc>> = positions
            .lock()
            .expect("log positions are never poisoned")
            .get(&pod)
            .and_then(|time| DateTime::parse_from_rfc3339(time).ok())
            .map(|time| time.with_timezone(&Utc));
        let since = last.map(|time| time.to_rfc3339_opts(SecondsFormat::Secs, true));
        let mut response = match kube.follow(&pod, &container, since.as_deref()).await {
            Ok(response) => response,
            Err(_) => {
                failures += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };
        failures = 0;
        let mut pending = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            pending.extend_from_slice(&chunk);
            while let Some(end) = pending.iter().position(|byte| *byte == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
                let Some((time, text)) = timestamped(&line) else {
                    continue;
                };
                // The log restarts at a whole second; skip what was recorded.
                if last.is_some_and(|last| time <= last) {
                    continue;
                }
                telemetry.output(&key, "stdout", crate::proxy::output::redact(text, &redact));
                positions
                    .lock()
                    .expect("log positions are never poisoned")
                    .insert(
                        pod.clone(),
                        time.to_rfc3339_opts(SecondsFormat::Nanos, true),
                    );
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Split a line that `timestamps=true` prefixed with its time.
fn timestamped(line: &str) -> Option<(DateTime<Utc>, &str)> {
    let (time, text) = line.split_once(' ').unwrap_or((line, ""));
    let time = DateTime::parse_from_rfc3339(time).ok()?.with_timezone(&Utc);
    Some((time, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_timestamps_from_lines() {
        let (time, text) =
            timestamped("2026-10-15T12:00:00.123456789Z {\"event\": \"purchase\"}").unwrap();
        assert_eq!(text, "{\"event\": \"purchase\"}");
        assert_eq!(time.timestamp_subsec_nanos(), 123_456_789);
        let (_, empty) = timestamped("2026-10-15T12:00:00Z").unwrap();
        assert_eq!(empty, "");
        assert!(timestamped("not a timestamp").is_none());
    }
}
