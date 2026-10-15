//! Containers on this machine: the app's default and each arm, run with
//! `docker`. Each listens on a private port of 127.0.0.1, restarts with
//! backoff when it exits, and gets traffic only while it is ready. A new
//! image replaces a container without dropping requests: the new container
//! starts and becomes ready, takes the traffic, and then the old one stops.

use std::{
    collections::{HashMap, HashSet},
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use regex::Regex;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::watch,
    task::JoinHandle,
};

use super::{
    assign::{Key, Target},
    output,
    telemetry::Telemetry,
};
use crate::image;

/// How long a container may take to become ready.
const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a stopping container may take before it is killed.
const STOP_SECONDS: u32 = 10;
/// A container that ran this long before exiting restarts without delay.
const STABLE_AFTER: Duration = Duration::from_secs(60);
/// The label every container of this proxy's app carries.
const APP_LABEL: &str = "statespace.com/app";
/// The label that names a container's default or arm.
const TARGET_LABEL: &str = "statespace.com/target";
/// The label that names a container's image.
const IMAGE_LABEL: &str = "statespace.com/image";
/// The label that names the proxy, by the address it listens on, so two
/// proxies of one app on a machine keep to their own containers.
const PROXY_LABEL: &str = "statespace.com/proxy";

/// How every container starts and what the proxy keeps of its output.
pub struct Options {
    pub app: String,
    /// The address the proxy listens on.
    pub listen: String,
    pub health: Option<String>,
    pub port: Option<u16>,
    pub volumes: Vec<String>,
    pub memory: Option<String>,
    pub cpus: Option<String>,
    /// The proxy's own environment, which every container inherits.
    pub environment: Vec<(String, String)>,
    /// What every arm gets on top, and the default does not.
    pub variant_env: Vec<(String, String)>,
    pub redact: Vec<Regex>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Starting,
    Ready,
    Failed,
}

struct Slot {
    image: watch::Sender<String>,
    stop: watch::Sender<bool>,
    state: State,
    /// `http://127.0.0.1:<port>` while ready.
    address: Option<String>,
    container: Option<String>,
    /// The image the serving container runs.
    running: Option<String>,
    restarts: u32,
    error: Option<String>,
}

/// A container the proxy found when it started, from an earlier run.
struct Found {
    id: String,
    image: String,
    running: bool,
}

/// How serving from a container ended.
enum Running {
    Exited(Option<i64>),
    Replaced,
    Stopped,
}

/// How waiting for a new container ended.
enum Readiness {
    Checked(anyhow::Result<()>),
    Stopped,
    Replaced,
}

/// A running container.
struct Instance {
    id: String,
    address: String,
    logs: Vec<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct Supervisor {
    slots: Arc<Mutex<HashMap<Target, Slot>>>,
    tasks: Arc<Mutex<Vec<JoinHandle<()>>>>,
    found: Arc<Mutex<HashMap<String, Found>>>,
    telemetry: Arc<Telemetry>,
    options: Arc<Options>,
}

impl Supervisor {
    /// A supervisor that first adopts the app's containers from an earlier
    /// run of the proxy, so a crash does not restart what still runs.
    pub async fn new(telemetry: Arc<Telemetry>, options: Options) -> anyhow::Result<Self> {
        let listed = docker(&[
            "ps",
            "--all",
            "--filter",
            &format!("label={APP_LABEL}={}", options.app),
            "--filter",
            &format!("label={PROXY_LABEL}={}", options.listen),
            "--format",
            &format!(
                "{{{{.ID}}}}\t{{{{.Label \"{TARGET_LABEL}\"}}}}\t{{{{.Label \"{IMAGE_LABEL}\"}}}}\t{{{{.State}}}}"
            ),
        ])
        .await
        .context("could not list containers; is Docker running, and may this user use it?")?;
        let mut found = HashMap::new();
        for row in listed.lines() {
            let fields: Vec<&str> = row.split('\t').collect();
            if let [id, target, image, state] = fields[..] {
                found.insert(
                    target.to_owned(),
                    Found {
                        id: id.to_owned(),
                        image: image.to_owned(),
                        running: state == "running",
                    },
                );
            }
        }
        Ok(Self {
            slots: Arc::default(),
            tasks: Arc::default(),
            found: Arc::new(Mutex::new(found)),
            telemetry,
            options: Arc::new(options),
        })
    }

    /// Run `target` from `image`. A target that runs another image switches
    /// to this one once its new container is ready.
    pub fn ensure(&self, target: Target, image: String) {
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        if let Some(slot) = slots.get(&target) {
            slot.image.send_if_modified(|current| {
                let changed = *current != image;
                current.clone_from(&image);
                changed
            });
            return;
        }
        let (image_sender, images) = watch::channel(image);
        let (stop, stopped) = watch::channel(false);
        slots.insert(
            target.clone(),
            Slot {
                image: image_sender,
                stop,
                state: State::Starting,
                address: None,
                container: None,
                running: None,
                restarts: 0,
                error: None,
            },
        );
        let supervisor = self.clone();
        let task = tokio::spawn(async move {
            supervisor.supervise(target, images, stopped).await;
        });
        self.tasks
            .lock()
            .expect("tasks are never poisoned")
            .push(task);
    }

    /// Stop every arm not in `desired`. The default always keeps running.
    pub fn retain(&self, desired: &HashSet<Key>) {
        let mut slots = self.slots.lock().expect("slots are never poisoned");
        slots.retain(|target, slot| {
            let keep = match target {
                Target::Default => true,
                Target::Arm(key) => desired.contains(key),
            };
            if !keep {
                let _ = slot.stop.send(true);
            }
            keep
        });
    }

    /// Remove the containers of an earlier run that no target adopted.
    pub async fn remove_strays(&self) {
        let strays: Vec<String> = self
            .found
            .lock()
            .expect("found containers are never poisoned")
            .drain()
            .map(|(_, found)| found.id)
            .collect();
        for id in strays {
            eprintln!("[ssp] removing container {id}, which no arm uses");
            let _ = docker(&["rm", "--force", &id]).await;
        }
    }

    /// Where a target that is ready takes traffic.
    pub fn address(&self, target: &Target) -> Option<String> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .get(target)
            .filter(|slot| slot.state == State::Ready)
            .and_then(|slot| slot.address.clone())
    }

    /// The image the ready default runs.
    pub fn default_image(&self) -> Option<String> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .get(&Target::Default)
            .filter(|slot| slot.state == State::Ready)
            .and_then(|slot| slot.running.clone())
    }

    /// Mark a target unready after a failed request. It becomes ready again
    /// when a check passes, or when its container restarts.
    pub fn suspect(&self, target: &Target) {
        let address = {
            let mut slots = self.slots.lock().expect("slots are never poisoned");
            let Some(slot) = slots
                .get_mut(target)
                .filter(|slot| slot.state == State::Ready)
            else {
                return;
            };
            slot.state = State::Starting;
            slot.address.clone()
        };
        let supervisor = self.clone();
        let target = target.clone();
        tokio::spawn(async move {
            let Some(address) = address else { return };
            for _ in 0..600 {
                if supervisor.check(&address).await {
                    supervisor.update(&target, |slot| {
                        if slot.state == State::Starting
                            && slot.address.as_deref() == Some(address.as_str())
                        {
                            slot.state = State::Ready;
                        }
                    });
                    return;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        });
    }

    /// Each container's state, for reports.
    pub fn snapshot(&self) -> Vec<Value> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        let mut arms: Vec<_> = slots
            .iter()
            .map(|(target, slot)| {
                json!({
                    "target": target.to_string(),
                    "image": slot.running.as_deref().unwrap_or(slot.image.borrow().as_str()),
                    "state": format!("{:?}", slot.state).to_lowercase(),
                    "restarts": slot.restarts,
                    "error": slot.error,
                })
            })
            .collect();
        arms.sort_by_key(|arm| arm["target"].to_string());
        arms
    }

    /// The containers of the arms, for resource sampling.
    pub fn arm_containers(&self) -> Vec<(Key, String)> {
        let slots = self.slots.lock().expect("slots are never poisoned");
        slots
            .iter()
            .filter_map(|(target, slot)| match target {
                Target::Arm(key) => Some((key.clone(), slot.container.clone()?)),
                Target::Default => None,
            })
            .collect()
    }

    /// Stop every container and wait for them to be removed.
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

    fn update(&self, target: &Target, change: impl FnOnce(&mut Slot)) {
        if let Some(slot) = self
            .slots
            .lock()
            .expect("slots are never poisoned")
            .get_mut(target)
        {
            change(slot);
        }
    }

    fn lifecycle(&self, target: &Target, event: &str, data: Value) {
        if let Target::Arm(key) = target {
            self.telemetry.lifecycle(key, event, data);
        }
    }

    async fn supervise(
        &self,
        target: Target,
        mut images: watch::Receiver<String>,
        mut stopped: watch::Receiver<bool>,
    ) {
        let mut serving: Option<Instance> = None;
        let mut delay = Duration::from_secs(1);
        loop {
            if *stopped.borrow() {
                break;
            }
            let image = images.borrow_and_update().clone();
            let started = Instant::now();
            let starting = tokio::select! {
                result = self.start(&target, &image) => Some(result),
                _ = stopped.wait_for(|stop| *stop) => None,
            };
            let instance = match starting {
                None => break,
                Some(Ok(instance)) => instance,
                Some(Err(error)) => {
                    eprintln!("[ssp] could not start {target}: {error:#}");
                    self.lifecycle(&target, "failed", json!({"error": format!("{error:#}")}));
                    if serving.is_none() {
                        self.update(&target, |slot| {
                            slot.state = State::Failed;
                            slot.error = Some(format!("{error:#}"));
                        });
                    }
                    if !self.pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                    continue;
                }
            };
            self.update(&target, |slot| {
                if serving.is_none() {
                    slot.container = Some(instance.id.clone());
                }
            });
            self.lifecycle(
                &target,
                "started",
                json!({"container": instance.id, "image": image}),
            );
            let ready = tokio::select! {
                ready = self.wait_ready(&instance) => Readiness::Checked(ready),
                _ = stopped.wait_for(|stop| *stop) => Readiness::Stopped,
                _ = images.changed() => Readiness::Replaced,
            };
            match ready {
                Readiness::Stopped => {
                    remove(instance).await;
                    break;
                }
                // A newer image replaces this one before it serves.
                Readiness::Replaced => {
                    remove(instance).await;
                    continue;
                }
                Readiness::Checked(Err(error)) => {
                    eprintln!("[ssp] {target} did not become ready: {error:#}");
                    self.update(&target, |slot| slot.error = Some(format!("{error:#}")));
                    self.lifecycle(&target, "failed", json!({"error": format!("{error:#}")}));
                    remove(instance).await;
                    if !self.pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                    continue;
                }
                Readiness::Checked(Ok(())) => {}
            }
            eprintln!("[ssp] {target} is ready at {}", instance.address);
            self.lifecycle(
                &target,
                "ready",
                json!({"seconds": started.elapsed().as_secs_f64()}),
            );
            self.update(&target, |slot| {
                slot.state = State::Ready;
                slot.error = None;
                slot.address = Some(instance.address.clone());
                slot.container = Some(instance.id.clone());
                slot.running = Some(image.clone());
            });
            // The new container serves; the one it replaces stops.
            if let Some(previous) = serving.replace(instance) {
                remove(previous).await;
            }
            let id = serving.as_ref().map(|instance| instance.id.clone());
            let running = tokio::select! {
                code = wait(id.as_deref().unwrap_or_default()) => Running::Exited(code),
                _ = images.changed() => Running::Replaced,
                _ = stopped.wait_for(|stop| *stop) => Running::Stopped,
            };
            match running {
                Running::Replaced => {}
                Running::Stopped => break,
                Running::Exited(code) => {
                    serving.take();
                    self.update(&target, |slot| {
                        slot.state = State::Starting;
                        slot.address = None;
                        slot.container = None;
                        slot.restarts += 1;
                    });
                    self.lifecycle(
                        &target,
                        "exited",
                        json!({"code": code, "seconds": started.elapsed().as_secs_f64()}),
                    );
                    if started.elapsed() > STABLE_AFTER {
                        delay = Duration::from_secs(1);
                    }
                    eprintln!("[ssp] {target} exited; restarting in {}s", delay.as_secs());
                    if let Some(id) = &id {
                        let _ = docker(&["rm", "--force", id]).await;
                    }
                    if !self.pause(&mut delay, &mut images, &mut stopped).await {
                        break;
                    }
                }
            }
        }
        if let Some(instance) = serving.take() {
            remove(instance).await;
        }
        self.lifecycle(&target, "stopped", json!({}));
    }

    /// Wait before starting again, doubling the delay up to a minute. A new
    /// image skips the wait. Returns false when the target must stop.
    async fn pause(
        &self,
        delay: &mut Duration,
        images: &mut watch::Receiver<String>,
        stopped: &mut watch::Receiver<bool>,
    ) -> bool {
        let waited = tokio::select! {
            _ = tokio::time::sleep(*delay) => true,
            _ = images.changed() => true,
            _ = stopped.wait_for(|stop| *stop) => false,
        };
        *delay = (*delay * 2).min(Duration::from_secs(60));
        waited
    }

    /// Start a container for `target`, or adopt one an earlier run left.
    async fn start(&self, target: &Target, image: &str) -> anyhow::Result<Instance> {
        let label = target_label(target);
        let found = self
            .found
            .lock()
            .expect("found containers are never poisoned")
            .remove(&label);
        let port = self.container_port(image).await?;
        if let Some(found) = found {
            if found.running && found.image == image {
                eprintln!("[ssp] adopting {target} from container {}", found.id);
                let address = published(&found.id, port).await?;
                let logs = self.follow(target, &found.id, true);
                return Ok(Instance {
                    id: found.id,
                    address,
                    logs,
                });
            }
            let _ = docker(&["rm", "--force", &found.id]).await;
        }
        let name = format!(
            "ssp-{}-{}-{:06x}",
            self.options.app,
            sanitize(&label),
            rand::random::<u32>() & 0xff_ffff
        );
        let mut command = Command::new("docker");
        command
            // An init process forwards SIGTERM to apps that run as PID 1.
            .args(["run", "--detach", "--init", "--name", &name])
            .args(["--label", &format!("{APP_LABEL}={}", self.options.app)])
            .args(["--label", &format!("{TARGET_LABEL}={label}")])
            .args(["--label", &format!("{PROXY_LABEL}={}", self.options.listen)])
            .args(["--label", &format!("{IMAGE_LABEL}={image}")])
            .args(["--publish", &format!("127.0.0.1::{port}")]);
        // Values travel in the environment of `docker` itself, so they never
        // appear in a process list.
        command.env_clear();
        for variable in [
            "PATH",
            "HOME",
            "DOCKER_HOST",
            "DOCKER_CONFIG",
            "DOCKER_CONTEXT",
        ] {
            if let Ok(value) = std::env::var(variable) {
                command.env(variable, value);
            }
        }
        let mut environment: Vec<(String, String)> = self.options.environment.clone();
        if let Target::Arm(_) = target {
            environment.extend(self.options.variant_env.iter().cloned());
        }
        environment.extend(statespace_environment(&self.options.app, target, port));
        let mut passed = HashSet::new();
        for (name, value) in environment.iter().rev() {
            if passed.insert(name.clone()) {
                command.arg("--env").arg(name);
                command.env(name, value);
            }
        }
        for volume in &self.options.volumes {
            command.args(["--volume", volume]);
        }
        if let Some(memory) = &self.options.memory {
            command.args(["--memory", memory]);
        }
        if let Some(cpus) = &self.options.cpus {
            command.args(["--cpus", cpus]);
        }
        command.arg(image);
        let output = command.output().await.context("could not run docker")?;
        if !output.status.success() {
            bail!(
                "docker run failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let address = match published(&id, port).await {
            Ok(address) => address,
            Err(error) => {
                let _ = docker(&["rm", "--force", &id]).await;
                return Err(error);
            }
        };
        let logs = self.follow(target, &id, false);
        Ok(Instance { id, address, logs })
    }

    /// The port the app listens on in its container: `--port`, else the
    /// image's first exposed TCP port. Pulls the image when it is missing.
    async fn container_port(&self, image: &str) -> anyhow::Result<u16> {
        let exposed = match inspect(image).await {
            Ok(inspected) => inspected,
            Err(_) => {
                eprintln!("[ssp] pulling {image}");
                docker(&["pull", "--quiet", image])
                    .await
                    .with_context(|| format!("could not pull {image}"))?;
                inspect(image).await?
            }
        };
        if let Some(port) = self.options.port {
            return Ok(port);
        }
        let mut ports: Vec<u16> = exposed["Config"]["ExposedPorts"]
            .as_object()
            .into_iter()
            .flat_map(|ports| ports.keys())
            .filter_map(|port| port.strip_suffix("/tcp")?.parse().ok())
            .collect();
        ports.sort_unstable();
        ports
            .first()
            .copied()
            .with_context(|| format!("{image} exposes no TCP port; pass --port"))
    }

    /// Pass a container's output through, and record each line of an arm.
    fn follow(&self, target: &Target, id: &str, adopted: bool) -> Vec<JoinHandle<()>> {
        let mut command = Command::new("docker");
        command.args(["logs", "--follow"]);
        if adopted {
            // Lines from before the proxy restarted were recorded already.
            command.args(["--since", "1s"]);
        }
        command
            .arg(id)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let Ok(mut child) = command.spawn() else {
            eprintln!("[ssp] could not follow the output of {target}");
            return Vec::new();
        };
        let mut tasks = Vec::new();
        let streams: [(
            &'static str,
            Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>>,
        ); 2] =
            [
                (
                    "stdout",
                    child.stdout.take().map(|stream| {
                        Box::new(stream) as Box<dyn tokio::io::AsyncRead + Send + Unpin>
                    }),
                ),
                (
                    "stderr",
                    child.stderr.take().map(|stream| {
                        Box::new(stream) as Box<dyn tokio::io::AsyncRead + Send + Unpin>
                    }),
                ),
            ];
        for (stream, reader) in streams {
            let Some(reader) = reader else { continue };
            tasks.push(tokio::spawn(forward(
                reader,
                target.clone(),
                stream,
                self.telemetry.clone(),
                self.options.clone(),
            )));
        }
        tasks.push(tokio::spawn(async move {
            let _ = child.wait().await;
        }));
        tasks
    }

    async fn wait_ready(&self, instance: &Instance) -> anyhow::Result<()> {
        let deadline = Instant::now() + READY_TIMEOUT;
        while Instant::now() < deadline {
            if self.check(&instance.address).await {
                return Ok(());
            }
            let state = docker(&["inspect", "--format", "{{.State.Status}}", &instance.id])
                .await
                .unwrap_or_default();
            if matches!(state.trim(), "exited" | "dead") {
                bail!("it exited before it was ready");
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        bail!(
            "it did not answer at {} within {}s",
            instance.address,
            READY_TIMEOUT.as_secs()
        )
    }

    /// Whether the container at `address` answers its health check.
    async fn check(&self, address: &str) -> bool {
        match &self.options.health {
            None => {
                let socket = address.trim_start_matches("http://");
                tokio::net::TcpStream::connect(socket).await.is_ok()
            }
            Some(path) => reqwest::Client::new()
                .get(format!("{address}{path}"))
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success()),
        }
    }
}

/// What every container learns about itself.
fn statespace_environment(app: &str, target: &Target, port: u16) -> Vec<(String, String)> {
    let mut environment = vec![
        ("PORT".into(), port.to_string()),
        ("STATESPACE_APP".into(), app.to_owned()),
    ];
    match target {
        Target::Default => environment.push(("STATESPACE_VARIANT".into(), "default".into())),
        Target::Arm(key) => {
            environment.push(("STATESPACE_VARIANT".into(), key.arm.clone()));
            environment.push(("STATESPACE_EXPERIMENT".into(), key.experiment.clone()));
            if let Some(version) = key.version {
                environment.push(("STATESPACE_VARIANT_VERSION".into(), version.to_string()));
            }
        }
    }
    environment
}

/// The environment of the proxy that containers inherit: everything but
/// the proxy's own settings and what describes this machine or session.
pub fn inherited_environment() -> Vec<(String, String)> {
    const MACHINE: [&str; 26] = [
        "PATH",
        "HOME",
        "HOSTNAME",
        "USER",
        "LOGNAME",
        "SHELL",
        "PWD",
        "OLDPWD",
        "SHLVL",
        "TERM",
        "TMPDIR",
        "MAIL",
        "_",
        "INVOCATION_ID",
        "JOURNAL_STREAM",
        "NOTIFY_SOCKET",
        "MAINPID",
        "MANAGERPID",
        "WATCHDOG_PID",
        "WATCHDOG_USEC",
        "RUNTIME_DIRECTORY",
        "STATE_DIRECTORY",
        "CACHE_DIRECTORY",
        "LOGS_DIRECTORY",
        "CONFIGURATION_DIRECTORY",
        "CREDENTIALS_DIRECTORY",
    ];
    const PREFIXES: [&str; 6] = [
        "SSP_",
        "STATESPACE_",
        "DOCKER_",
        "XDG_",
        "SYSTEMD_",
        "LISTEN_",
    ];
    std::env::vars()
        .filter(|(name, _)| {
            !MACHINE.contains(&name.as_str())
                && !PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        })
        .collect()
}

/// The digest of an image Docker has: the registry digest of its repository,
/// else its local ID.
pub async fn digest(image: &str) -> anyhow::Result<String> {
    let inspected = inspect(image).await?;
    let digests: Vec<String> = inspected["RepoDigests"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|digest| Some(digest.as_str()?.to_owned()))
        .collect();
    let wanted = image::repository(image);
    if let Some(digest) = digests
        .iter()
        .find(|digest| image::repository(digest) == wanted && image::is_digest(digest))
    {
        return Ok(digest.clone());
    }
    if image::is_digest(image) {
        return Ok(image.to_owned());
    }
    inspected["Id"]
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("docker reported no ID for {image}"))
}

/// Pull `image` unless Docker has it.
pub async fn pull(image: &str) -> anyhow::Result<()> {
    if inspect(image).await.is_ok() {
        return Ok(());
    }
    docker(&["pull", "--quiet", image])
        .await
        .with_context(|| format!("could not pull {image}"))
        .map(drop)
}

async fn inspect(image: &str) -> anyhow::Result<Value> {
    let output = docker(&["image", "inspect", "--format", "{{json .}}", image]).await?;
    Ok(serde_json::from_str(output.trim())?)
}

/// CPU and memory of running containers, by ID.
pub async fn stats(ids: &[String]) -> HashMap<String, Value> {
    if ids.is_empty() {
        return HashMap::new();
    }
    let mut arguments = vec!["stats", "--no-stream", "--format", "{{json .}}"];
    arguments.extend(ids.iter().map(String::as_str));
    let Ok(output) = docker(&arguments).await else {
        return HashMap::new();
    };
    output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|row| {
            let id = row["ID"].as_str()?.to_owned();
            let cpu = row["CPUPerc"]
                .as_str()?
                .trim_end_matches('%')
                .parse::<f64>()
                .ok()?;
            let memory = row["MemUsage"].as_str()?.split('/').next().and_then(bytes);
            Some((id, json!({"cpu_percent": cpu, "memory_bytes": memory})))
        })
        .collect()
}

/// Bytes in a size Docker prints, such as `12.5MiB`.
fn bytes(size: &str) -> Option<u64> {
    let size = size.trim();
    let split = size
        .find(|character: char| character.is_ascii_alphabetic())
        .unwrap_or(size.len());
    let (number, unit) = size.split_at(split);
    let number: f64 = number.trim().parse().ok()?;
    let scale = match unit {
        "B" | "" => 1.0,
        "kB" | "KB" => 1e3,
        "MB" => 1e6,
        "GB" => 1e9,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        _ => return None,
    };
    Some((number * scale) as u64)
}

/// Where Docker published a container's port on 127.0.0.1.
async fn published(id: &str, port: u16) -> anyhow::Result<String> {
    let output = docker(&["port", id, &format!("{port}/tcp")]).await?;
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("127.0.0.1:"))
        .map(|host_port| format!("http://127.0.0.1:{host_port}"))
        .with_context(|| format!("docker published no port for container {id}"))
}

/// Wait for a container to stop; its exit code, if Docker knows it.
async fn wait(id: &str) -> Option<i64> {
    docker(&["wait", id]).await.ok()?.trim().parse().ok()
}

/// Stop a container, give it time to finish, and remove it.
async fn remove(instance: Instance) {
    let _ = docker(&["stop", "--time", &STOP_SECONDS.to_string(), &instance.id]).await;
    let _ = docker(&["rm", "--force", &instance.id]).await;
    for task in instance.logs {
        task.abort();
    }
}

/// Run `docker` and return its standard output.
async fn docker(arguments: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("docker")
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .context("could not run docker")?;
    if !output.status.success() {
        bail!(
            "docker {} failed: {}",
            arguments.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A stable label for a target, which names its containers across restarts.
fn target_label(target: &Target) -> String {
    match target {
        Target::Default => "default".into(),
        Target::Arm(key) => format!(
            "{}/{}/{}/{}",
            key.experiment,
            key.experiment_version,
            key.arm,
            key.version
                .map_or_else(|| "-".into(), |version| version.to_string())
        ),
    }
}

/// Letters, digits, dots, and dashes, as container names allow.
fn sanitize(label: &str) -> String {
    label
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .take(80)
        .collect()
}

/// Pass a container's lines through. An arm's lines are also recorded.
async fn forward(
    reader: Box<dyn tokio::io::AsyncRead + Send + Unpin>,
    target: Target,
    stream: &'static str,
    telemetry: Arc<Telemetry>,
    options: Arc<Options>,
) {
    let mut lines = BufReader::new(reader).lines();
    let prefix = match &target {
        Target::Default => String::new(),
        Target::Arm(key) => format!("[{key}] "),
    };
    while let Ok(Some(line)) = lines.next_line().await {
        if stream == "stdout" {
            println!("{prefix}{line}");
        } else {
            eprintln!("{prefix}{line}");
        }
        if let Target::Arm(key) = &target {
            telemetry.output(key, stream, output::redact(&line, &options.redact));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_name_targets_stably() {
        let key = Key {
            experiment: "price".into(),
            experiment_version: 2,
            arm: "lower-price".into(),
            version: Some(3),
        };
        assert_eq!(target_label(&Target::Arm(key)), "price/2/lower-price/3");
        assert_eq!(target_label(&Target::Default), "default");
        assert_eq!(sanitize("price/2/control/-"), "price-2-control--");
    }

    #[test]
    fn reads_docker_sizes() {
        assert_eq!(bytes("12MiB"), Some(12 * 1024 * 1024));
        assert_eq!(bytes("1.5GiB"), Some(1_610_612_736));
        assert_eq!(bytes("512B"), Some(512));
        assert_eq!(bytes("3kB"), Some(3000));
        assert_eq!(bytes("lots"), None);
    }

    #[test]
    fn containers_learn_what_they_run() {
        let key = Key {
            experiment: "price".into(),
            experiment_version: 1,
            arm: "control".into(),
            version: None,
        };
        let environment = statespace_environment("shop", &Target::Arm(key), 8000);
        assert!(environment.contains(&("PORT".into(), "8000".into())));
        assert!(environment.contains(&("STATESPACE_VARIANT".into(), "control".into())));
        assert!(environment.contains(&("STATESPACE_EXPERIMENT".into(), "price".into())));
    }
}
