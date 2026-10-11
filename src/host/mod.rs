//! `ssp host`: run an app and its variants as processes behind a proxy
//! that assigns each subject to one of them.

mod assign;
mod output;
mod processes;
mod proxy;
mod remote;
mod telemetry;

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use regex::Regex;
use serde_json::json;
use sysinfo::{Pid, ProcessesToUpdate, System};

use self::{
    assign::{Key, Routing},
    processes::{Bundle, Options, Source, Supervisor, label},
    proxy::Shared,
    remote::Remote,
    telemetry::Telemetry,
};
use crate::{
    api::Api,
    cli::{Capture, HostArgs},
    signing,
};

/// How long the host waits for control before it listens anyway.
const CONTROL_TIMEOUT: Duration = Duration::from_secs(120);
/// How long in-flight requests may finish after a shutdown signal.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run(api: Api, arguments: HostArgs) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(host(api, arguments))
}

async fn host(api: Api, arguments: HostArgs) -> anyhow::Result<()> {
    if !arguments.command.contains("{port}") {
        bail!("--command must contain {{port}}, where the app must listen");
    }
    let variant_env = arguments
        .variant_env
        .iter()
        .map(|pair| {
            pair.split_once('=')
                .filter(|(name, _)| !name.is_empty())
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .with_context(|| format!("--variant-env {pair:?} must look like NAME=VALUE"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let redact = arguments
        .redact
        .iter()
        .map(|pattern| {
            Regex::new(pattern).with_context(|| format!("--redact {pattern:?} is invalid"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let trusted = arguments
        .trust_keys
        .iter()
        .map(|key| signing::public_key(key))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let local_variants = arguments
        .variants
        .iter()
        .map(|pair| {
            let (name, path) = pair
                .split_once('=')
                .filter(|(name, _)| !name.is_empty() && *name != "control")
                .with_context(|| format!("--variant {pair:?} must look like NAME=PATH"))?;
            let path = PathBuf::from(path)
                .canonicalize()
                .with_context(|| format!("{path} does not exist"))?;
            Ok((name.to_owned(), path))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let remote = match &arguments.app {
        Some(app) if !arguments.local => {
            let token = api
                .token()
                .context("ssp host needs a runtime key; set SSP_API_KEY or run `ssp login`")?;
            Some(Remote::new(api.endpoint(), token.to_owned(), app.clone())?)
        }
        _ => None,
    };
    let cache = match arguments.cache.clone() {
        Some(cache) => cache,
        None => dirs::cache_dir()
            .context("set --cache; the user cache directory is unavailable")?
            .join("statespace/variants"),
    };
    let host_id = format!(
        "{}-{:06x}",
        System::host_name().unwrap_or_else(|| "host".into()),
        rand::random::<u32>() & 0xff_ffff
    );

    let telemetry = Arc::new(Telemetry::start(host_id.clone(), remote.clone()));
    let supervisor = Supervisor::new(
        telemetry.clone(),
        Options {
            app: arguments.app.clone(),
            command: arguments.command.clone(),
            health: arguments.health.clone(),
            variant_env,
            capture_stdout: arguments.capture.contains(&Capture::Stdout),
            capture_stderr: arguments.capture.contains(&Capture::Stderr),
            redact,
        },
    );
    let shared = Arc::new(Shared::new(
        supervisor.clone(),
        telemetry.clone(),
        arguments.local,
        arguments.capture.contains(&Capture::Http),
        Duration::from_secs(arguments.stale_after),
    ));

    supervisor.ensure(
        Key::control(),
        Source::Directory(std::env::current_dir()?),
        None,
    );
    for (name, path) in local_variants {
        let key = Key {
            name,
            version: None,
        };
        supervisor.ensure(key, Source::Directory(path), None);
    }
    if let Some(remote) = &remote {
        tokio::spawn(poll(
            remote.clone(),
            shared.clone(),
            cache,
            trusted,
            arguments.max_variants,
        ));
        tokio::spawn(report(
            remote.clone(),
            shared.clone(),
            host_id.clone(),
            arguments.max_variants,
        ));
    }
    tokio::spawn(measure(shared.clone()));

    let started = Instant::now();
    while supervisor.port(&Key::control()).is_none() && started.elapsed() < CONTROL_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let listener = tokio::net::TcpListener::bind(&arguments.listen)
        .await
        .with_context(|| format!("could not listen on {}", arguments.listen))?;
    eprintln!(
        "[ssp] listening on {} as {host_id}{}",
        arguments.listen,
        if arguments.local { " (local)" } else { "" }
    );
    let (stop, stopped) = tokio::sync::watch::channel(false);
    let server = tokio::spawn(proxy::serve(listener, shared.clone(), stopped));

    shutdown_signal().await?;
    eprintln!("[ssp] stopping");
    let _ = stop.send(true);
    let _ = server.await;
    let draining = Instant::now();
    while shared.in_flight.load(Ordering::SeqCst) > 0 && draining.elapsed() < DRAIN_TIMEOUT {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    supervisor.shutdown().await;
    telemetry.flush_http();
    // Give the last events a moment to leave.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

/// Keep the app's configuration and processes current.
async fn poll(
    remote: Remote,
    shared: Arc<Shared>,
    cache: PathBuf,
    trusted: Vec<ed25519_dalek::VerifyingKey>,
    max_variants: u32,
) {
    let mut interval = Duration::from_secs(10);
    let mut failing = false;
    let mut reported: HashSet<String> = HashSet::new();
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
                    if reported.insert(error.clone()) {
                        eprintln!("[ssp] serving control instead of {error}");
                    }
                }
                let mut desired = HashSet::new();
                for (index, (key, variant)) in routing.variants().into_iter().enumerate() {
                    if index >= max_variants as usize {
                        let message =
                            format!("{} exceeds --max-variants {max_variants}", label(&key));
                        if reported.insert(message.clone()) {
                            eprintln!("[ssp] {message}; its subjects get control");
                        }
                        continue;
                    }
                    desired.insert(key.clone());
                    shared.supervisor.ensure(
                        key,
                        Source::Bundle(Box::new(Bundle {
                            remote: remote.clone(),
                            variant: variant.clone(),
                            cache: cache.clone(),
                            trusted: trusted.clone(),
                        })),
                        variant.command.clone(),
                    );
                }
                shared.supervisor.retain(&desired);
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

/// Tell Statespace what this host runs and how much it can run.
async fn report(remote: Remote, shared: Arc<Shared>, host_id: String, max_variants: u32) {
    loop {
        let age = shared
            .current
            .read()
            .expect("routing is never poisoned")
            .fetched
            .map(|fetched| fetched.elapsed().as_secs());
        let report = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "processes": shared.supervisor.snapshot(),
            "dropped_events": shared.telemetry.dropped(),
            "config_age_seconds": age,
        });
        if let Err(error) = remote.report(&host_id, max_variants, report).await {
            eprintln!("[ssp] could not report to Statespace: {error:#}");
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// Every minute, send HTTP summaries and each process's CPU and memory.
async fn measure(shared: Arc<Shared>) {
    let mut system = System::new();
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        shared.telemetry.flush_http();
        system.refresh_processes(ProcessesToUpdate::All, true);
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for (pid, process) in system.processes() {
            if let Some(parent) = process.parent() {
                children.entry(parent).or_default().push(*pid);
            }
        }
        for (key, pid) in shared.supervisor.pids() {
            let (mut cpu, mut memory, mut count) = (0.0_f64, 0_u64, 0_u64);
            let mut pending = vec![Pid::from_u32(pid)];
            while let Some(pid) = pending.pop() {
                if let Some(process) = system.process(pid) {
                    cpu += f64::from(process.cpu_usage());
                    memory += process.memory();
                    count += 1;
                }
                pending.extend(children.get(&pid).into_iter().flatten());
            }
            shared.telemetry.resources(
                &key,
                json!({"cpu_percent": cpu, "memory_bytes": memory, "processes": count}),
            );
        }
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> anyhow::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn shutdown_signal() -> anyhow::Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}
