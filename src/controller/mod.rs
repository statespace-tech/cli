//! `ssp controller`: run the arms of an app's experiments on Kubernetes.
//!
//! The controller keeps one Deployment and Service per arm: a copy of the
//! default's Deployment that runs the arm's image. It reports where each
//! arm runs so the proxies beside the default can reach it, and records
//! every line the arms print. It never touches the default's Deployment.

mod kube;
mod logs;
mod spec;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, bail};
use serde_json::{Value, json};

use self::{kube::Kube, logs::Followers};
use crate::{
    api::Api,
    cli::ControllerArgs,
    proxy::{
        self,
        assign::{self, Arm, Key, Routing},
        remote::Remote,
        telemetry::Telemetry,
    },
    signing,
};

/// How many arms the controller runs unless `--max-variants` says otherwise.
const MAX_ARMS: u32 = 10;

pub fn run(api: Api, arguments: ControllerArgs) -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(controller(api, arguments))
}

/// What the controller needs on each pass.
struct Controller {
    kube: Kube,
    remote: Remote,
    app: String,
    deployment: String,
    container: Option<String>,
    trusted: Vec<ed25519_dalek::VerifyingKey>,
    variant_env: Vec<(String, String)>,
    max_arms: u32,
    id: String,
    /// The controller's own Deployment, which owns every arm.
    owner: Option<Value>,
    state: State,
    /// Problems already printed, so each is printed once.
    noted: HashSet<String>,
}

async fn controller(api: Api, arguments: ControllerArgs) -> anyhow::Result<()> {
    let shared = &arguments.shared;
    let token = api
        .token()
        .context("ssp controller needs a token with the controller preset; set SSP_TOKEN")?;
    let remote = Remote::new(api.endpoint(), token.to_owned(), shared.app.clone())?;
    let kube = Kube::connect(arguments.namespace.clone())?;
    let trusted = arguments
        .trust_keys
        .iter()
        .map(|key| signing::public_key(key))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let id = proxy::instance_id();
    let telemetry = Arc::new(Telemetry::start(id.clone(), Some(remote.clone())));
    let owner = match std::env::var("STATESPACE_CONTROLLER") {
        Ok(name) => {
            kube.get(&format!("{}/{name}", kube.path("apps/v1", "deployments")))
                .await?
        }
        Err(_) => None,
    };
    if owner.is_none() {
        eprintln!(
            "[ssp] STATESPACE_CONTROLLER does not name this controller's Deployment, so \
             deleting it leaves its arms running"
        );
    }
    let state = State::load(&kube, &arguments.deployment).await;
    let followers = Followers::new(
        kube.clone(),
        telemetry.clone(),
        proxy::patterns(&shared.redact)?,
        state.logs.clone(),
    );
    let mut controller = Controller {
        kube,
        remote,
        app: shared.app.clone(),
        deployment: arguments.deployment.clone(),
        container: arguments.container.clone(),
        trusted,
        variant_env: proxy::pairs(&shared.variant_env, "--variant-env")?,
        max_arms: shared.max_variants.unwrap_or(MAX_ARMS),
        id: id.clone(),
        owner,
        state,
        noted: HashSet::new(),
    };
    eprintln!(
        "[ssp] controller {id} runs the arms of {} in namespace {}",
        controller.deployment, controller.kube.namespace
    );
    let mut reported: HashSet<String> = HashSet::new();
    let mut interval = Duration::from_secs(10);
    loop {
        let pass = tokio::select! {
            pass = controller.reconcile(&followers) => pass,
            _ = stop() => break,
        };
        match pass {
            Ok(seconds) => interval = Duration::from_secs(seconds.clamp(1, 300)),
            Err(error) => {
                let message = format!("{error:#}");
                if reported.insert(message.clone()) {
                    eprintln!("[ssp] {message}");
                }
            }
        }
        controller
            .state
            .save(&controller.kube, &controller.deployment, &followers)
            .await;
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = stop() => break,
        }
    }
    eprintln!("[ssp] stopping; the arms keep running until their experiments stop");
    controller
        .state
        .save(&controller.kube, &controller.deployment, &followers)
        .await;
    telemetry.flush_http();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    Ok(())
}

impl Controller {
    fn note(&mut self, message: String) {
        if self.noted.insert(message.clone()) {
            eprintln!("[ssp] {message}");
        }
    }

    /// One pass: make the arms match the running experiments, then report.
    /// Returns how long to wait before the next pass.
    async fn reconcile(&mut self, followers: &Followers) -> anyhow::Result<u64> {
        let config = self.remote.config().await?;
        let (routing, errors) = Routing::compile(&config);
        for error in errors {
            self.note(format!("serving the default instead of {error}"));
        }
        let deployments = self.kube.path("apps/v1", "deployments");
        let services = self.kube.path("v1", "services");
        let default = self
            .kube
            .get(&format!("{deployments}/{}", self.deployment))
            .await?
            .with_context(|| format!("Deployment {} does not exist", self.deployment))?;
        let pod = &default["spec"]["template"]["spec"];
        let container = spec::app_container(pod, self.container.as_deref())?;
        let container_name = pod["containers"][container]["name"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let port = spec::container_port(&pod["containers"][container]).with_context(|| {
            format!(
                "the {container_name} container of {} declares no containerPort",
                self.deployment
            )
        })?;
        let default_replicas = default["spec"]["replicas"].as_u64().unwrap_or(1);
        if let Some(digest) = self.default_digest(&default, &container_name).await? {
            self.state.defaults.insert(digest.clone());
            self.state.current = Some(digest);
        }

        let selector = format!(
            "{}={},{}={}",
            spec::MANAGED_BY.0,
            spec::MANAGED_BY.1,
            spec::APP_LABEL,
            self.app
        );
        let existing: HashMap<String, Value> = self
            .kube
            .list(&deployments, &selector)
            .await?
            .into_iter()
            .filter_map(|deployment| {
                Some((
                    deployment["metadata"]["name"].as_str()?.to_owned(),
                    deployment,
                ))
            })
            .collect();
        let mut desired: HashSet<String> = HashSet::new();
        let mut reports = Vec::new();
        for (key, image) in self.arms(&routing).await {
            if desired.len() >= self.max_arms as usize {
                self.note(format!(
                    "{key} exceeds --max-variants {}; its subjects get the default",
                    self.max_arms
                ));
                continue;
            }
            let name = spec::arm_name(&self.deployment, &key);
            let config = routing
                .experiments
                .iter()
                .map(|experiment| &experiment.config)
                .find(|config| config.name == key.experiment)
                .context("the arm's experiment is missing")?;
            let replicas = spec::replicas(default_replicas, assign::share(config, &key));
            let mut environment = self.variant_env.clone();
            environment.extend(environment_of(&self.app, &key));
            let deployment = spec::arm_deployment(
                &default,
                container,
                &self.app,
                &name,
                &key,
                &image,
                replicas,
                &environment,
                self.owner.as_ref(),
            )?;
            let ready = match existing.get(&name) {
                Some(current) if spec::current(current, &deployment) => {
                    current["status"]["readyReplicas"].as_u64().unwrap_or(0) > 0
                }
                Some(current) => {
                    let mut replacement = deployment.clone();
                    replacement["metadata"]["resourceVersion"] =
                        current["metadata"]["resourceVersion"].clone();
                    self.kube
                        .replace(&format!("{deployments}/{name}"), &replacement)
                        .await?;
                    eprintln!("[ssp] updated {key} as Deployment {name}");
                    current["status"]["readyReplicas"].as_u64().unwrap_or(0) > 0
                }
                None => {
                    self.kube.create(&deployments, &deployment).await?;
                    eprintln!("[ssp] started {key} as Deployment {name}");
                    false
                }
            };
            let service = spec::arm_service(&self.app, &name, port, self.owner.as_ref());
            match self.kube.get(&format!("{services}/{name}")).await? {
                Some(current) if spec::current(&current, &service) => {}
                Some(_) => {
                    self.kube.delete(&format!("{services}/{name}")).await?;
                    self.kube.create(&services, &service).await?;
                }
                None => {
                    self.kube.create(&services, &service).await?;
                }
            }
            reports.push(json!({
                "experiment": key.experiment,
                "version": key.experiment_version,
                "name": key.arm,
                "variant_version": key.version,
                "image": image,
                "address": format!("http://{name}.{}.svc:{port}", self.kube.namespace),
                "ready": ready,
                "replicas": replicas,
            }));
            desired.insert(name);
        }
        for name in existing.keys().filter(|name| !desired.contains(*name)) {
            self.kube.delete(&format!("{deployments}/{name}")).await?;
            self.kube.delete(&format!("{services}/{name}")).await?;
            eprintln!("[ssp] removed Deployment {name}, which no running experiment uses");
        }
        self.remote.report_arms(&self.id, reports.clone()).await?;
        self.remote
            .report(
                &self.id,
                "controller",
                self.max_arms,
                json!({"deployment": self.deployment, "arms": reports}),
            )
            .await?;
        let pods = self
            .kube
            .list(&self.kube.path("v1", "pods"), &selector)
            .await?;
        followers.follow(&pods, &container_name);
        Ok(config.poll_interval_seconds)
    }

    /// Each arm this controller may run, with its image. A control arm runs
    /// the pinned default image, pinning the current default when the
    /// version has none; a variant runs only when a trusted key signed it.
    async fn arms(&mut self, routing: &Routing) -> Vec<(Key, String)> {
        let mut arms = Vec::new();
        for (key, arm) in routing.arms() {
            match arm {
                Arm::Control(control) => {
                    let pinned = match (&control.image, &self.state.current) {
                        (Some(image), _) => image.clone(),
                        (None, Some(current)) => match self
                            .remote
                            .pin_control(&key.experiment, key.experiment_version, current)
                            .await
                        {
                            Ok(pin) => pin.image,
                            Err(error) => {
                                self.note(format!("could not pin the control of {key}: {error:#}"));
                                continue;
                            }
                        },
                        (None, None) => continue,
                    };
                    if !self.state.defaults.contains(&pinned) {
                        self.note(format!(
                            "{key} runs {pinned}, which this controller never saw as the \
                             default; its subjects get the default"
                        ));
                        continue;
                    }
                    arms.push((key, pinned));
                }
                Arm::Variant(variant) => {
                    let message = signing::message(
                        &self.app,
                        &variant.name,
                        &variant.image,
                        variant.commit.as_deref(),
                    );
                    if let Err(error) = signing::verify(&self.trusted, &variant.signature, &message)
                    {
                        self.note(format!("{key} cannot run: {error:#}"));
                        continue;
                    }
                    arms.push((key, variant.image.clone()));
                }
            }
        }
        arms
    }

    /// The digest the default's ready pods run, when they all run the
    /// Deployment's current image.
    async fn default_digest(
        &self,
        default: &Value,
        container: &str,
    ) -> anyhow::Result<Option<String>> {
        let Some(labels) = default["spec"]["selector"]["matchLabels"].as_object() else {
            bail!("the default's Deployment selects pods without matchLabels");
        };
        let selector = labels
            .iter()
            .map(|(name, value)| format!("{name}={}", value.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(",");
        let template_image = default["spec"]["template"]["spec"]["containers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|candidate| candidate["name"] == container)
            .and_then(|candidate| candidate["image"].as_str())
            .unwrap_or_default()
            .to_owned();
        let pods = self
            .kube
            .list(&self.kube.path("v1", "pods"), &selector)
            .await?;
        let mut digests = HashSet::new();
        for pod in &pods {
            let runs_template = pod["spec"]["containers"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|candidate| {
                    candidate["name"] == container && candidate["image"] == template_image
                });
            if !runs_template {
                continue;
            }
            for status in pod["status"]["containerStatuses"]
                .as_array()
                .into_iter()
                .flatten()
            {
                if status["name"] == container
                    && status["ready"] == true
                    && let Some(digest) = status["imageID"].as_str().and_then(image_digest)
                {
                    digests.insert(digest);
                }
            }
        }
        // During a rollout, the pods of one template can differ by digest.
        Ok((digests.len() == 1).then(|| digests.into_iter().next().expect("one digest")))
    }
}

/// `repository@sha256:…` from a container status's `imageID`, which may
/// carry a scheme such as `docker-pullable://`.
fn image_digest(image_id: &str) -> Option<String> {
    let reference = image_id.rsplit("://").next().unwrap_or(image_id);
    crate::image::is_digest(reference).then(|| reference.to_owned())
}

/// What every arm's container learns about itself.
fn environment_of(app: &str, key: &Key) -> Vec<(String, String)> {
    let mut environment = vec![
        ("STATESPACE_APP".to_owned(), app.to_owned()),
        ("STATESPACE_VARIANT".to_owned(), key.arm.clone()),
        ("STATESPACE_EXPERIMENT".to_owned(), key.experiment.clone()),
    ];
    if let Some(version) = key.version {
        environment.push(("STATESPACE_VARIANT_VERSION".to_owned(), version.to_string()));
    }
    environment
}

/// What survives a restart of the controller, in a ConfigMap: the default
/// images it saw, and how far it read each arm pod's log.
struct State {
    defaults: HashSet<String>,
    current: Option<String>,
    logs: Arc<std::sync::Mutex<HashMap<String, String>>>,
    saved: Option<String>,
}

impl State {
    const LIMIT: usize = 100;

    fn name(deployment: &str) -> String {
        format!("{deployment}-statespace")
    }

    async fn load(kube: &Kube, deployment: &str) -> Self {
        let path = format!(
            "{}/{}",
            kube.path("v1", "configmaps"),
            Self::name(deployment)
        );
        let data = match kube.get(&path).await {
            Ok(Some(map)) => map["data"].clone(),
            Ok(None) => Value::Null,
            Err(error) => {
                eprintln!("[ssp] could not read the controller's state: {error:#}");
                Value::Null
            }
        };
        let defaults = data["defaults"]
            .as_str()
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
        let logs = data["logs"]
            .as_str()
            .and_then(|logs| serde_json::from_str(logs).ok())
            .unwrap_or_default();
        Self {
            defaults,
            current: None,
            logs: Arc::new(std::sync::Mutex::new(logs)),
            saved: None,
        }
    }

    /// Save the state when it changed. Logs of pods that are gone are dropped.
    async fn save(&mut self, kube: &Kube, deployment: &str, followers: &Followers) {
        let mut defaults: Vec<&String> = self.defaults.iter().collect();
        defaults.sort();
        let excess = defaults.len().saturating_sub(Self::LIMIT);
        let defaults: Vec<&str> = defaults[excess..]
            .iter()
            .map(|digest| digest.as_str())
            .collect();
        let logs = {
            let mut logs = self.logs.lock().expect("log positions are never poisoned");
            let live = followers.pods();
            logs.retain(|pod, _| live.contains(pod));
            serde_json::to_string(&*logs).unwrap_or_default()
        };
        let data = json!({"defaults": defaults.join("\n"), "logs": logs});
        let serialized = data.to_string();
        if self.saved.as_deref() == Some(serialized.as_str()) {
            return;
        }
        let name = Self::name(deployment);
        let path = kube.path("v1", "configmaps");
        let map = json!({
            "apiVersion": "v1",
            "kind": "ConfigMap",
            "metadata": {
                "name": name,
                "labels": {spec::MANAGED_BY.0: spec::MANAGED_BY.1},
            },
            "data": data,
        });
        let saved = match kube.get(&format!("{path}/{name}")).await {
            Ok(Some(existing)) => {
                let mut replacement = map;
                replacement["metadata"]["resourceVersion"] =
                    existing["metadata"]["resourceVersion"].clone();
                kube.replace(&format!("{path}/{name}"), &replacement)
                    .await
                    .map(drop)
            }
            Ok(None) => kube.create(&path, &map).await.map(drop),
            Err(error) => Err(error),
        };
        match saved {
            Ok(()) => self.saved = Some(serialized),
            Err(error) => eprintln!("[ssp] could not save the controller's state: {error:#}"),
        }
    }
}

#[cfg(unix)]
async fn stop() {
    use tokio::signal::unix::{SignalKind, signal};

    match signal(SignalKind::terminate()) {
        Ok(mut terminate) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
        }
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(not(unix))]
async fn stop() {
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_digests_from_container_statuses() {
        let digest = format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64));
        assert_eq!(
            image_digest(&format!("docker-pullable://{digest}")).as_deref(),
            Some(digest.as_str())
        );
        assert_eq!(image_digest(&digest).as_deref(), Some(digest.as_str()));
        assert_eq!(image_digest("sha256:abc"), None);
    }
}
