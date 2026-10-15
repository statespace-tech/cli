//! `ssp proxy`: a reverse proxy that assigns each request's subject and
//! forwards the request to the app's default or to an experiment's arm.
//!
//! With `--runtime docker`, the proxy also runs the default and every arm
//! as containers on this machine. With `--runtime kubernetes`, it runs
//! beside the default in its pod and forwards to arms that `ssp controller`
//! runs.

pub mod assign;
mod docker;
pub mod output;
pub mod remote;
mod server;
pub mod telemetry;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, RwLock, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use regex::Regex;
use serde_json::json;

use self::{
    assign::{Arm, Routing, Target},
    docker::Supervisor,
    remote::Remote,
    server::{Shared, Upstreams},
    telemetry::Telemetry,
};
use crate::{
    api::Api,
    cli::{ProxyArgs, Runtime},
    signing,
};

/// How long the proxy waits for the default before it listens anyway.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(180);
/// How long in-flight requests may finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);
/// How many arms a proxy runs unless `--max-variants` says otherwise.
const MAX_ARMS: u32 = 4;

pub fn run(api: Api, arguments: ProxyArgs) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(proxy(api, arguments))
}

/// `NAME=VALUE` pairs, as `--variant-env` takes them.
pub fn pairs(values: &[String], flag: &str) -> anyhow::Result<Vec<(String, String)>> {
    values
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .with_context(|| format!("{flag} {pair:?} must look like NAME=VALUE"))
        })
        .collect()
}

pub fn patterns(values: &[String]) -> anyhow::Result<Vec<Regex>> {
    values
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("--redact {pattern:?} is invalid"))
        })
        .collect()
}

/// A name for this proxy or controller in reports: the machine and a random suffix.
pub fn instance_id() -> String {
    format!(
        "{}-{:06x}",
        sysinfo::System::host_name().unwrap_or_else(|| "host".into()),
        rand::random::<u32>() & 0xff_ffff
    )
}

async fn proxy(api: Api, arguments: ProxyArgs) -> anyhow::Result<()> {
    let shared_arguments = &arguments.shared;
    let app = shared_arguments.app.clone();
    let token = api
        .token()
        .context("ssp proxy needs a token with the runtime preset; set SSP_TOKEN")?;
    let remote = Remote::new(api.endpoint(), token.to_owned(), app.clone())?;
    let id = instance_id();
    let telemetry = Arc::new(Telemetry::start(id.clone(), Some(remote.clone())));
    let redact = patterns(&shared_arguments.redact)?;
    let variant_env = pairs(&shared_arguments.variant_env, "--variant-env")?;
    let max_arms = shared_arguments.max_variants.unwrap_or(MAX_ARMS);

    let (upstreams, default_image) = match arguments.runtime {
        Runtime::Docker => {
            if arguments.default.is_some() {
                bail!("--default is for --runtime kubernetes; with docker, pass --image");
            }
            if arguments.trust_keys.is_empty() {
                bail!("pass --trust-key with the public key from `ssp keygen`");
            }
            let image = default_image(&arguments)?
                .context("pass --image or --image-file with the app's default image")?;
            let supervisor = Supervisor::new(
                telemetry.clone(),
                docker::Options {
                    app: app.clone(),
                    listen: arguments.listen.clone(),
                    health: shared_arguments.health.clone(),
                    port: arguments.port,
                    volumes: arguments.volumes.clone(),
                    memory: arguments.memory.clone(),
                    cpus: arguments.cpus.clone(),
                    environment: docker::inherited_environment(),
                    variant_env,
                    redact,
                },
            )
            .await?;
            docker::pull(&image).await?;
            supervisor.ensure(Target::Default, image.clone());
            (Upstreams::Docker(supervisor), Some(image))
        }
        Runtime::Kubernetes => {
            let default = arguments
                .default
                .clone()
                .context("pass --default with where the app listens in its pod, such as http://localhost:8000")?;
            if arguments.image.is_some()
                || arguments.image_file.is_some()
                || arguments.port.is_some()
                || !arguments.volumes.is_empty()
            {
                bail!(
                    "on Kubernetes, the default's Deployment sets its image, port, and volumes; \
                     pass only --default"
                );
            }
            (
                Upstreams::Kubernetes {
                    default,
                    arms: RwLock::default(),
                    suspects: std::sync::Mutex::default(),
                },
                None,
            )
        }
    };
    let trusted = arguments
        .trust_keys
        .iter()
        .map(|key| signing::public_key(key))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let shared = Arc::new(Shared::new(
        upstreams,
        telemetry.clone(),
        Duration::from_secs(shared_arguments.stale_after),
    ));
    let history = Arc::new(History::load(&app));

    tokio::spawn(poll(
        remote.clone(),
        shared.clone(),
        trusted,
        max_arms,
        history.clone(),
    ));
    tokio::spawn(report(
        remote.clone(),
        shared.clone(),
        id.clone(),
        max_arms,
        arguments.runtime,
    ));
    tokio::spawn(measure(shared.clone()));

    if let Upstreams::Docker(supervisor) = &shared.upstreams {
        let started = Instant::now();
        while supervisor.address(&Target::Default).is_none() && started.elapsed() < DEFAULT_TIMEOUT
        {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    let listener = tokio::net::TcpListener::bind(&arguments.listen)
        .await
        .with_context(|| format!("could not listen on {}", arguments.listen))?;
    eprintln!("[ssp] proxy {id} listening on {}", arguments.listen);
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(server::serve(listener, shared.clone(), stopped));

    loop {
        match signal().await? {
            Signal::Stop => break,
            Signal::Reload => reload(&arguments, &shared, default_image.as_deref()).await,
        }
    }
    eprintln!("[ssp] stopping");
    let _ = stop.send(true);
    let _ = server.await;
    let draining = Instant::now();
    while shared.in_flight.load(Ordering::SeqCst) > 0 && draining.elapsed() < DRAIN_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if let Upstreams::Docker(supervisor) = &shared.upstreams {
        supervisor.shutdown().await;
    }
    telemetry.flush_http();
    // Give the last events a moment to leave.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

/// The default image: `--image`, or the contents of `--image-file`.
fn default_image(arguments: &ProxyArgs) -> anyhow::Result<Option<String>> {
    if let Some(image) = &arguments.image {
        return Ok(Some(image.trim().to_owned()));
    }
    let Some(path) = &arguments.image_file else {
        return Ok(None);
    };
    let image = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?
        .trim()
        .to_owned();
    if image.is_empty() {
        bail!("{} holds no image", path.display());
    }
    Ok(Some(image))
}

/// Swap the default for the image `--image-file` now holds, once the new
/// container is ready.
async fn reload(arguments: &ProxyArgs, shared: &Shared, started_with: Option<&str>) {
    let Upstreams::Docker(supervisor) = &shared.upstreams else {
        eprintln!("[ssp] nothing to reload: Kubernetes deploys the default");
        return;
    };
    if arguments.image_file.is_none() {
        eprintln!("[ssp] nothing to reload: the default comes from --image {started_with:?}");
        return;
    }
    let image = match default_image(arguments) {
        Ok(Some(image)) => image,
        Ok(None) => return,
        Err(error) => {
            eprintln!("[ssp] could not reload: {error:#}");
            return;
        }
    };
    eprintln!("[ssp] switching the default to {image}");
    if let Err(error) = docker::pull(&image).await {
        eprintln!("[ssp] could not reload: {error:#}");
        return;
    }
    supervisor.ensure(Target::Default, image);
}

/// The default images this proxy ran, by digest. A control arm runs only an
/// image this proxy ran as its default, so Statespace cannot choose what
/// runs here.
struct History {
    path: Option<PathBuf>,
    digests: std::sync::Mutex<Vec<String>>,
}

impl History {
    const LIMIT: usize = 100;

    fn load(app: &str) -> Self {
        let path = std::env::var_os("STATESPACE_STATE")
            .map(PathBuf::from)
            .or_else(|| dirs::data_local_dir().map(|directory| directory.join("statespace")))
            .map(|directory| directory.join(format!("{app}-defaults.json")));
        let digests = path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            path,
            digests: std::sync::Mutex::new(digests),
        }
    }

    fn contains(&self, digest: &str) -> bool {
        self.digests
            .lock()
            .expect("history is never poisoned")
            .iter()
            .any(|known| known == digest)
    }

    fn add(&self, digest: &str) {
        let mut digests = self.digests.lock().expect("history is never poisoned");
        if digests.iter().any(|known| known == digest) {
            return;
        }
        digests.push(digest.to_owned());
        let excess = digests.len().saturating_sub(Self::LIMIT);
        digests.drain(..excess);
        let Some(path) = &self.path else { return };
        let saved = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(path, json!(*digests).to_string()));
        if let Err(error) = saved {
            eprintln!(
                "[ssp] could not save the default's digest to {}: {error}",
                path.display()
            );
        }
    }
}

/// Keep the app's configuration and arms current.
async fn poll(
    remote: Remote,
    shared: Arc<Shared>,
    trusted: Vec<ed25519_dalek::VerifyingKey>,
    max_arms: u32,
    history: Arc<History>,
) {
    let mut interval = Duration::from_secs(10);
    let mut failing = false;
    let mut reported: HashSet<String> = HashSet::new();
    let mut note = |message: String| {
        if reported.insert(message.clone()) {
            eprintln!("[ssp] {message}");
        }
    };
    let mut digests: HashMap<String, String> = HashMap::new();
    let mut strays_removed = false;
    loop {
        match remote.config().await {
            Ok(config) => {
                if failing {
                    eprintln!("[ssp] reached Statespace again");
                    failing = false;
                }
                interval = Duration::from_secs(config.poll_interval_seconds.clamp(1, 300));
                let (routing, errors) = Routing::compile(&config);
                for error in errors {
                    note(format!("serving the default instead of {error}"));
                }
                match &shared.upstreams {
                    Upstreams::Docker(supervisor) => {
                        let default = match supervisor.default_image() {
                            Some(image) => match digests.get(&image) {
                                Some(digest) => Some(digest.clone()),
                                None => match docker::digest(&image).await {
                                    Ok(digest) => {
                                        history.add(&digest);
                                        digests.insert(image, digest.clone());
                                        Some(digest)
                                    }
                                    Err(error) => {
                                        note(format!(
                                            "could not read the default's digest: {error:#}"
                                        ));
                                        None
                                    }
                                },
                            },
                            None => None,
                        };
                        let desired = arms(
                            &remote,
                            &routing,
                            supervisor,
                            &trusted,
                            max_arms,
                            default.as_deref(),
                            &history,
                            &mut note,
                        )
                        .await;
                        supervisor.retain(&desired);
                        if !strays_removed && default.is_some() {
                            supervisor.remove_strays().await;
                            strays_removed = true;
                        }
                    }
                    Upstreams::Kubernetes { arms, .. } => {
                        let mut ready = HashMap::new();
                        for (key, arm) in routing.arms() {
                            let (address, up) = match arm {
                                Arm::Control(control) => (&control.address, control.ready),
                                Arm::Variant(variant) => (&variant.address, variant.ready),
                            };
                            if let (Some(address), true) = (address, up) {
                                ready.insert(key, address.clone());
                            }
                        }
                        *arms.write().expect("arms are never poisoned") = ready;
                    }
                }
                let mut current = shared.current.write().expect("routing is never poisoned");
                current.routing = routing;
                current.fetched = Some(Instant::now());
            }
            Err(error) => {
                if !failing {
                    eprintln!("[ssp] could not reach Statespace: {error:#}");
                    failing = true;
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}

/// Start each arm this proxy may run, and return them. A control arm runs
/// the pinned default image, pinning this proxy's default when the version
/// has none; a variant runs only when a trusted key signed it.
#[allow(clippy::too_many_arguments)]
async fn arms(
    remote: &Remote,
    routing: &Routing,
    supervisor: &Supervisor,
    trusted: &[ed25519_dalek::VerifyingKey],
    max_arms: u32,
    default: Option<&str>,
    history: &History,
    note: &mut impl FnMut(String),
) -> HashSet<assign::Key> {
    let mut desired = HashSet::new();
    for (key, arm) in routing.arms() {
        let image = match arm {
            Arm::Control(control) => {
                let pinned = match (&control.image, default) {
                    (Some(image), _) => image.clone(),
                    (None, Some(default)) => {
                        match remote
                            .pin_control(&key.experiment, key.experiment_version, default)
                            .await
                        {
                            Ok(pin) => pin.image,
                            Err(error) => {
                                note(format!("could not pin the control of {key}: {error:#}"));
                                continue;
                            }
                        }
                    }
                    // Until the default is ready, its digest is unknown.
                    (None, None) => continue,
                };
                if !history.contains(&pinned) {
                    note(format!(
                        "{key} runs {pinned}, which this proxy never ran as its default; \
                         its subjects get the default"
                    ));
                    continue;
                }
                pinned
            }
            Arm::Variant(variant) => {
                let message = signing::message(
                    remote.app(),
                    &variant.name,
                    &variant.image,
                    variant.commit.as_deref(),
                );
                if let Err(error) = signing::verify(trusted, &variant.signature, &message) {
                    note(format!(
                        "{key} cannot run: {error:#}; its subjects get the default"
                    ));
                    continue;
                }
                variant.image.clone()
            }
        };
        if desired.len() >= max_arms as usize {
            note(format!(
                "{key} exceeds --max-variants {max_arms}; its subjects get the default"
            ));
            continue;
        }
        supervisor.ensure(Target::Arm(key.clone()), image);
        desired.insert(key);
    }
    desired
}

/// Tell Statespace what this proxy runs and how much it can run.
async fn report(remote: Remote, shared: Arc<Shared>, id: String, max_arms: u32, runtime: Runtime) {
    loop {
        let age = shared
            .current
            .read()
            .expect("routing is never poisoned")
            .fetched
            .map(|fetched| fetched.elapsed().as_secs());
        let arms = match &shared.upstreams {
            Upstreams::Docker(supervisor) => json!(supervisor.snapshot()),
            Upstreams::Kubernetes { arms, .. } => json!(
                arms.read()
                    .expect("arms are never poisoned")
                    .keys()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            ),
        };
        let report = json!({
            "runtime": match runtime {
                Runtime::Docker => "docker",
                Runtime::Kubernetes => "kubernetes",
            },
            "arms": arms,
            "dropped_events": shared.telemetry.dropped(),
            "config_age_seconds": age,
        });
        if let Err(error) = remote.report(&id, "proxy", max_arms, report).await {
            eprintln!("[ssp] could not report to Statespace: {error:#}");
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Every minute, send each arm's HTTP summary and, on Docker, its CPU and memory.
async fn measure(shared: Arc<Shared>) {
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        shared.telemetry.flush_http();
        let Upstreams::Docker(supervisor) = &shared.upstreams else {
            continue;
        };
        let containers = supervisor.arm_containers();
        let ids: Vec<String> = containers.iter().map(|(_, id)| id.clone()).collect();
        let stats = docker::stats(&ids).await;
        for (key, id) in containers {
            let short: String = id.chars().take(12).collect();
            if let Some(data) = stats.get(&id).or_else(|| stats.get(&short)) {
                shared.telemetry.resources(&key, data.clone());
            }
        }
    }
}

enum Signal {
    Stop,
    Reload,
}

#[cfg(unix)]
async fn signal() -> anyhow::Result<Signal> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    let mut hangup = signal(SignalKind::hangup())?;
    Ok(tokio::select! {
        result = tokio::signal::ctrl_c() => { result?; Signal::Stop }
        _ = terminate.recv() => Signal::Stop,
        _ = hangup.recv() => Signal::Reload,
    })
}

#[cfg(not(unix))]
async fn signal() -> anyhow::Result<Signal> {
    tokio::signal::ctrl_c().await?;
    Ok(Signal::Stop)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variant_environment() {
        let parsed = pairs(&["A=b".into(), "C=d=e".into()], "--variant-env").unwrap();
        assert_eq!(
            parsed,
            vec![("A".into(), "b".into()), ("C".into(), "d=e".into())]
        );
        assert!(pairs(&["=x".into()], "--variant-env").is_err());
        assert!(pairs(&["NOVALUE".into()], "--variant-env").is_err());
    }

    #[test]
    fn remembers_default_images() {
        let directory = tempfile::tempdir().unwrap();
        // SAFETY: tests that read STATESPACE_STATE run in this one test.
        unsafe { std::env::set_var("STATESPACE_STATE", directory.path()) };
        let history = History::load("shop");
        assert!(!history.contains("ghcr.io/acme/shop@sha256:a"));
        history.add("ghcr.io/acme/shop@sha256:a");
        let reloaded = History::load("shop");
        assert!(reloaded.contains("ghcr.io/acme/shop@sha256:a"));
        assert!(!History::load("other").contains("ghcr.io/acme/shop@sha256:a"));
    }
}
