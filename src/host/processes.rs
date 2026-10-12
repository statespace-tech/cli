//! The processes a host runs: control and variants. Each runs the app's
//! start command on a private port, restarts with backoff when it exits,
//! and gets traffic only while it is ready.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use regex::Regex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::watch,
};

use super::{
    assign::{Key, VariantConfig},
    output::{self, Line},
    remote::Remote,
    telemetry::Telemetry,
};
use crate::{bundle, signing};

/// How long a process may take to become ready.
const READY_TIMEOUT: Duration = Duration::from_secs(120);
/// How long a stopping process may take before it is killed.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// A process that ran this long before exiting restarts without delay.
const STABLE_AFTER: Duration = Duration::from_secs(60);

/// How every process starts and what the host keeps of its output.
pub struct Options {
    pub app: Option<String>,
    pub command: String,
    pub health: Option<String>,
    pub variant_env: Vec<(String, String)>,
    pub capture_stdout: bool,
    pub capture_stderr: bool,
    pub redact: Vec<Regex>,
}

/// Where a process's files come from.
pub enum Source {
    /// A directory on this machine: control, or a local variant.
    Directory(PathBuf),
    /// A signed bundle that is downloaded, verified, and unpacked first.
    Bundle(Box<Bundle>),
}

pub struct Bundle {
    pub remote: Remote,
    pub variant: VariantConfig,
    pub cache: PathBuf,
    pub trusted: Vec<ed25519_dalek::VerifyingKey>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Preparing,
    Starting,
    Ready,
    Failed,
}

struct Slot {
    state: State,
    port: Option<u16>,
    pid: Option<u32>,
    restarts: u32,
    error: Option<String>,
    stop: watch::Sender<bool>,
}

#[derive(Clone)]
pub struct Supervisor {
    slots: Arc<Mutex<HashMap<Key, Slot>>>,
    tasks: Arc<Mutex<Vec<tokio::task::JoinHandle<()>>>>,
    telemetry: Arc<Telemetry>,
    options: Arc<Options>,
}

impl Supervisor {
    pub fn new(telemetry: Arc<Telemetry>, options: Options) -> Self {
        Self {
            slots: Arc::default(),
            tasks: Arc::default(),
            telemetry,
            options: Arc::new(options),
        }
    }

    /// Run `key` unless it already runs.
    pub fn ensure(&self, key: Key, source: Source, command: Option<String>) {
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        if slots.contains_key(&key) {
            return;
        }
        let (stop, stopped) = watch::channel(false);
        slots.insert(
            key.clone(),
            Slot {
                state: State::Preparing,
                port: None,
                pid: None,
                restarts: 0,
                error: None,
                stop,
            },
        );
        let supervisor = self.clone();
        let task = tokio::spawn(async move {
            supervisor.supervise(key, source, command, stopped).await;
        });
        self.tasks
            .lock()
            .expect("tasks are never poisoned")
            .push(task);
    }

    /// Stop every variant not in `desired`. Control always keeps running.
    pub fn retain(&self, desired: &HashSet<Key>) {
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        slots.retain(|key, slot| {
            let keep = key.is_control() || desired.contains(key);
            if !keep {
                let _ = slot.stop.send(true);
            }
            keep
        });
    }

    /// The port of a process that is ready for traffic.
    pub fn port(&self, key: &Key) -> Option<u16> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .get(key)
            .filter(|slot| slot.state == State::Ready)
            .and_then(|slot| slot.port)
    }

    /// Mark a process unready after a failed request. It becomes ready again
    /// when a check passes, or when it restarts.
    pub fn suspect(&self, key: &Key) {
        let port = {
            let mut slots = self.slots.lock().expect("slots are never poisoned");
            let Some(slot) = slots.get_mut(key).filter(|slot| slot.state == State::Ready) else {
                return;
            };
            slot.state = State::Starting;
            slot.port
        };
        let supervisor = self.clone();
        let key = key.clone();
        tokio::spawn(async move {
            let Some(port) = port else { return };
            for _ in 0..600 {
                if supervisor.check(port).await {
                    supervisor.update(&key, |slot| {
                        if slot.state == State::Starting && slot.port == Some(port) {
                            slot.state = State::Ready;
                        }
                    });
                    return;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    }

    /// Each process's state, for host reports.
    pub fn snapshot(&self) -> Vec<Value> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        let mut processes: Vec<_> = slots
            .iter()
            .map(|(key, slot)| {
                json!({
                    "variant": key.name,
                    "version": key.version,
                    "state": format!("{:?}", slot.state).to_lowercase(),
                    "restarts": slot.restarts,
                    "error": slot.error,
                })
            })
            .collect();
        processes.sort_by_key(|process| process["variant"].to_string());
        processes
    }

    /// The process IDs of ready processes, for resource sampling.
    pub fn pids(&self) -> Vec<(Key, u32)> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .iter()
            .filter_map(|(key, slot)| Some((key.clone(), slot.pid?)))
            .collect()
    }

    /// Stop every process and wait for them to exit.
    pub async fn shutdown(&self) {
        for slot in self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .values()
        {
            let _ = slot.stop.send(true);
        }
        let tasks = std::mem::take(&mut *self.tasks.lock().expect("tasks are never poisoned"));
        for task in tasks {
            let _ = task.await;
        }
    }

    fn update(&self, key: &Key, change: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .get_mut(key)
        {
            change(slot);
        }
    }

    async fn supervise(
        &self,
        key: Key,
        source: Source,
        command: Option<String>,
        mut stopped: watch::Receiver<bool>,
    ) {
        let directory = match prepare(&key, source).await {
            Ok(directory) => directory,
            Err(error) => {
                eprintln!("[ssp] {} cannot run: {error:#}", label(&key));
                self.telemetry
                    .lifecycle(&key, "failed", json!({"error": format!("{error:#}")}));
                self.update(&key, |slot| {
                    slot.state = State::Failed;
                    slot.error = Some(format!("{error:#}"));
                });
                return;
            }
        };
        let command = command.unwrap_or_else(|| self.options.command.clone());
        let mut delay = Duration::from_secs(1);
        loop {
            if *stopped.borrow() {
                return;
            }
            self.update(&key, |slot| slot.state = State::Starting);
            let started = Instant::now();
            let (mut child, port) = match self.spawn(&key, &directory, &command) {
                Ok(spawned) => spawned,
                Err(error) => {
                    eprintln!("[ssp] could not start {}: {error:#}", label(&key));
                    self.telemetry.lifecycle(
                        &key,
                        "failed",
                        json!({"error": format!("{error:#}")}),
                    );
                    self.update(&key, |slot| {
                        slot.state = State::Failed;
                        slot.error = Some(format!("{error:#}"));
                    });
                    return;
                }
            };
            let pid = child.id();
            self.update(&key, |slot| {
                slot.port = Some(port);
                slot.pid = pid;
            });
            self.telemetry
                .lifecycle(&key, "started", json!({"pid": pid, "port": port}));
            let ready = tokio::select! {
                ready = self.wait_ready(port, &mut child) => Some(ready),
                _ = stopped.wait_for(|stop| *stop) => None,
            };
            let exit = match ready {
                None => None,
                Some(Ok(())) => {
                    self.update(&key, |slot| {
                        slot.state = State::Ready;
                        slot.error = None;
                    });
                    self.telemetry.lifecycle(
                        &key,
                        "ready",
                        json!({"seconds": started.elapsed().as_secs_f64()}),
                    );
                    eprintln!("[ssp] {} is ready on port {port}", label(&key));
                    tokio::select! {
                        status = child.wait() => Some(status),
                        _ = stopped.wait_for(|stop| *stop) => None,
                    }
                }
                Some(Err(error)) => {
                    self.update(&key, |slot| slot.error = Some(format!("{error:#}")));
                    eprintln!("[ssp] {} did not become ready: {error:#}", label(&key));
                    if !matches!(child.try_wait(), Ok(Some(_))) {
                        terminate(&mut child).await;
                    }
                    Some(child.wait().await)
                }
            };
            let Some(status) = exit else {
                terminate(&mut child).await;
                self.telemetry.lifecycle(&key, "stopped", json!({}));
                return;
            };
            let status = status.ok();
            // Workers that outlive their parent would keep running unsupervised.
            kill_group(pid);
            self.update(&key, |slot| {
                slot.state = State::Starting;
                slot.port = None;
                slot.pid = None;
                slot.restarts += 1;
            });
            self.telemetry.lifecycle(
                &key,
                "exited",
                json!({
                    "code": status.and_then(|status| status.code()),
                    "signal": status.and_then(signal),
                    "seconds": started.elapsed().as_secs_f64(),
                }),
            );
            if started.elapsed() > STABLE_AFTER {
                delay = Duration::from_secs(1);
            }
            eprintln!(
                "[ssp] {} exited; restarting in {}s",
                label(&key),
                delay.as_secs()
            );
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = stopped.wait_for(|stop| *stop) => return,
            }
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    }

    fn spawn(&self, key: &Key, directory: &PathBuf, command: &str) -> anyhow::Result<(Child, u16)> {
        let port = free_port()?;
        let mut process = Command::new("sh");
        process
            .arg("-c")
            .arg(command.replace("{port}", &port.to_string()))
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        if !key.is_control() {
            for (name, value) in &self.options.variant_env {
                process.env(name, value);
            }
        }
        process.env("PORT", port.to_string());
        process.env("STATESPACE_VARIANT", &key.name);
        match key.version {
            Some(version) => process.env("STATESPACE_VARIANT_VERSION", version.to_string()),
            None => process.env_remove("STATESPACE_VARIANT_VERSION"),
        };
        if let Some(app) = &self.options.app {
            process.env("STATESPACE_APP", app);
        }
        // The host's own credentials are not the app's to use.
        process.env_remove("SSP_API_KEY");
        let mut child = process
            .spawn()
            .with_context(|| format!("could not run {command:?}"))?;
        for (stream, capture) in [
            ("stdout", self.options.capture_stdout),
            ("stderr", self.options.capture_stderr),
        ] {
            let reader: Box<dyn tokio::io::AsyncRead + Send + Unpin> = if stream == "stdout" {
                Box::new(child.stdout.take().context("stdout is piped")?)
            } else {
                Box::new(child.stderr.take().context("stderr is piped")?)
            };
            tokio::spawn(forward(
                reader,
                key.clone(),
                stream,
                capture,
                self.telemetry.clone(),
                self.options.clone(),
            ));
        }
        Ok((child, port))
    }

    async fn wait_ready(&self, port: u16, child: &mut Child) -> anyhow::Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = child.try_wait() {
                bail!("it exited with {status} before it was ready");
            }
            if self.check(port).await {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        bail!(
            "it did not accept connections on port {port} within {}s",
            READY_TIMEOUT.as_secs()
        )
    }

    /// Whether the process on `port` answers its health check.
    async fn check(&self, port: u16) -> bool {
        match &self.options.health {
            None => tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok(),
            Some(path) => reqwest::Client::new()
                .get(format!("http://127.0.0.1:{port}{path}"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success()),
        }
    }
}

/// The files of a process: a local directory, or a verified, unpacked bundle.
async fn prepare(key: &Key, source: Source) -> anyhow::Result<PathBuf> {
    let Bundle {
        remote,
        variant,
        cache,
        trusted,
    } = match source {
        Source::Directory(directory) => return Ok(directory),
        Source::Bundle(bundle) => *bundle,
    };
    // Check the signature before downloading anything.
    let message = signing::message(
        remote.app(),
        &key.name,
        &variant.sha256,
        variant.command.as_deref(),
        variant.commit.as_deref(),
    );
    signing::verify(&trusted, &variant.signature, &message)?;
    let directory = cache.join(&variant.sha256);
    if directory.is_dir() {
        return Ok(directory);
    }
    tokio::fs::create_dir_all(&cache).await?;
    let download = tempfile::NamedTempFile::new_in(&cache)?;
    eprintln!("[ssp] downloading {}", label(key));
    remote
        .download(&variant.sha256, variant.size, download.path())
        .await?;
    let destination = directory.clone();
    tokio::task::spawn_blocking(move || {
        let unpacked = bundle::unpack(download.path(), &destination);
        // Another host process may have unpacked the same bundle meanwhile.
        if unpacked.is_err() && destination.is_dir() {
            return Ok(());
        }
        unpacked
    })
    .await??;
    Ok(directory)
}

/// Pass a process's output through, record outcome lines, and keep captured lines.
async fn forward(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    key: Key,
    stream: &'static str,
    capture: bool,
    telemetry: Arc<Telemetry>,
    options: Arc<Options>,
) {
    let mut lines = BufReader::new(reader).lines();
    let prefix = if key.is_control() {
        String::new()
    } else {
        format!("[{}] ", label(&key))
    };
    while let Ok(Some(line)) = lines.next_line().await {
        if stream == "stdout" {
            println!("{prefix}{line}");
        } else {
            eprintln!("{prefix}{line}");
        }
        match output::classify(&line) {
            Line::Outcome(outcome) => telemetry.outcome(&key, outcome),
            Line::Invalid(reason) => {
                eprintln!("[ssp] ignored an outcome from {}: {reason}", label(&key))
            }
            Line::Plain => {}
        }
        if capture {
            telemetry.output(&key, stream, output::redact(&line, &options.redact));
        }
    }
}

/// Ask the process group to stop, then kill whatever is left of it.
async fn terminate(child: &mut Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: signalling a process group that this host started.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGTERM);
        }
        let _ = tokio::time::timeout(STOP_TIMEOUT, child.wait()).await;
        kill_group(Some(pid));
    }
    let _ = child.kill().await;
}

/// Kill every process left in the group that `pid` leads.
fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        // SAFETY: signalling a process group that this host started. The
        // group's ID stays reserved while any member is alive.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(unix)]
fn signal(status: std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn signal(_: std::process::ExitStatus) -> Option<i32> {
    None
}

fn free_port() -> anyhow::Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}

pub fn label(key: &Key) -> String {
    match key.version {
        Some(version) => format!("{}@{version}", key.name),
        None => key.name.clone(),
    }
}
