//! The Deployment and Service of an arm: a copy of the default's
//! Deployment that runs the arm's image instead.

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use crate::proxy::assign::Key;

/// The sidecar that runs `ssp proxy` beside the default. Arms never run it.
pub const PROXY_CONTAINER: &str = "statespace-proxy";
/// The label every object the controller creates carries.
pub const MANAGED_BY: (&str, &str) = ("app.kubernetes.io/managed-by", "ssp-controller");
pub const APP_LABEL: &str = "statespace.com/app";
/// The label that selects an arm's pods.
pub const ARM_LABEL: &str = "statespace.com/arm";
/// The annotation that names an arm.
pub const KEY_ANNOTATION: &str = "statespace.com/key";
/// The annotation that tells whether an object is current.
const HASH_ANNOTATION: &str = "statespace.com/spec-hash";

/// A name for an arm's Deployment and Service: lowercase, at most 63
/// characters, and different for each version of the experiment and arm.
pub fn arm_name(deployment: &str, key: &Key) -> String {
    let digest = Sha256::digest(
        format!(
            "{}/{}/{}/{:?}",
            key.experiment, key.experiment_version, key.arm, key.version
        )
        .as_bytes(),
    );
    let suffix: String = digest[..4]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let readable: String = format!("{deployment}-{}-{}", key.experiment, key.arm)
        .to_ascii_lowercase()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();
    let readable: String = readable.chars().take(63 - 9).collect();
    format!("{}-{suffix}", readable.trim_end_matches('-'))
}

/// The arm a pod or Deployment runs, from its annotation.
pub fn key_of(object: &Value) -> Option<Key> {
    let key: Value =
        serde_json::from_str(object["metadata"]["annotations"][KEY_ANNOTATION].as_str()?).ok()?;
    Some(Key {
        experiment: key["experiment"].as_str()?.to_owned(),
        experiment_version: u32::try_from(key["experiment_version"].as_u64()?).ok()?,
        arm: key["arm"].as_str()?.to_owned(),
        version: key["version"]
            .as_u64()
            .and_then(|version| u32::try_from(version).ok()),
    })
}

/// The index of the app's container in a pod spec: `wanted`, else the
/// first container other than the proxy sidecar.
pub fn app_container(pod: &Value, wanted: Option<&str>) -> anyhow::Result<usize> {
    let containers = pod["containers"]
        .as_array()
        .context("the default's pod template has no containers")?;
    let found = containers.iter().position(|container| {
        let name = container["name"].as_str().unwrap_or_default();
        match wanted {
            Some(wanted) => name == wanted,
            None => name != PROXY_CONTAINER,
        }
    });
    match (found, wanted) {
        (Some(index), _) => Ok(index),
        (None, Some(wanted)) => bail!("the default's pods have no container {wanted}"),
        (None, None) => bail!("the default's pods have no container besides {PROXY_CONTAINER}"),
    }
}

/// The first port the app's container declares.
pub fn container_port(container: &Value) -> Option<u16> {
    container["ports"]
        .as_array()?
        .iter()
        .filter(|port| {
            port["protocol"]
                .as_str()
                .is_none_or(|protocol| protocol == "TCP")
        })
        .find_map(|port| u16::try_from(port["containerPort"].as_u64()?).ok())
}

/// How many pods an arm gets: its share of the default's, at least one.
pub fn replicas(default_replicas: u64, share: f64) -> u64 {
    ((default_replicas as f64 * share).ceil() as u64).max(1)
}

/// What an arm's objects are labeled with.
fn labels(app: &str, name: &str) -> Value {
    json!({
        MANAGED_BY.0: MANAGED_BY.1,
        APP_LABEL: app,
        ARM_LABEL: name,
    })
}

/// The arm's copy of the default's Deployment. Its pods carry only the
/// arm's labels, so the default's Service never selects them, and run the
/// app's container without the proxy sidecar.
#[allow(clippy::too_many_arguments)]
pub fn arm_deployment(
    default: &Value,
    container: usize,
    app: &str,
    name: &str,
    key: &Key,
    image: &str,
    replicas: u64,
    environment: &[(String, String)],
    owner: Option<&Value>,
) -> anyhow::Result<Value> {
    let template = &default["spec"]["template"];
    let mut pod = template["spec"].clone();
    let containers = pod["containers"]
        .as_array_mut()
        .context("the default's pod template has no containers")?;
    let mut app_container = containers
        .get(container)
        .cloned()
        .context("the app's container is missing")?;
    app_container["image"] = json!(image);
    let mut variables: Vec<Value> = app_container["env"].as_array().cloned().unwrap_or_default();
    variables.retain(|variable| {
        let name = variable["name"].as_str().unwrap_or_default();
        !environment.iter().any(|(set, _)| set == name)
    });
    variables.extend(
        environment
            .iter()
            .map(|(name, value)| json!({"name": name, "value": value})),
    );
    app_container["env"] = json!(variables);
    let others: Vec<Value> = containers
        .iter()
        .enumerate()
        .filter(|(index, other)| {
            *index != container && other["name"].as_str() != Some(PROXY_CONTAINER)
        })
        .map(|(_, other)| other.clone())
        .collect();
    let mut all = vec![app_container];
    all.extend(others);
    pod["containers"] = json!(all);
    let mut annotations = template["metadata"]["annotations"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    annotations.insert(KEY_ANNOTATION.into(), json!(key_json(key)));
    let mut deployment = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": name,
            "labels": labels(app, name),
            "annotations": {
                KEY_ANNOTATION: key_json(key),
                "statespace.com/image": image,
            },
        },
        "spec": {
            "replicas": replicas,
            "selector": {"matchLabels": {ARM_LABEL: name}},
            "template": {
                "metadata": {"labels": labels(app, name), "annotations": annotations},
                "spec": pod,
            },
        },
    });
    if let Some(strategy) = default["spec"].get("strategy") {
        deployment["spec"]["strategy"] = strategy.clone();
    }
    owned(&mut deployment, owner);
    Ok(hashed(deployment))
}

/// The Service that reaches an arm's pods.
pub fn arm_service(app: &str, name: &str, port: u16, owner: Option<&Value>) -> Value {
    let mut service = json!({
        "apiVersion": "v1",
        "kind": "Service",
        "metadata": {"name": name, "labels": labels(app, name)},
        "spec": {
            "selector": {ARM_LABEL: name},
            "ports": [{"name": "http", "port": port, "targetPort": port, "protocol": "TCP"}],
        },
    });
    owned(&mut service, owner);
    hashed(service)
}

/// Whether `existing` was made from the same specification as `desired`.
pub fn current(existing: &Value, desired: &Value) -> bool {
    existing["metadata"]["annotations"][HASH_ANNOTATION]
        == desired["metadata"]["annotations"][HASH_ANNOTATION]
}

fn key_json(key: &Key) -> String {
    json!({
        "experiment": key.experiment,
        "experiment_version": key.experiment_version,
        "arm": key.arm,
        "version": key.version,
    })
    .to_string()
}

/// Let Kubernetes delete the object with the controller's Deployment.
fn owned(object: &mut Value, owner: Option<&Value>) {
    let Some(owner) = owner else { return };
    object["metadata"]["ownerReferences"] = json!([{
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "name": owner["metadata"]["name"],
        "uid": owner["metadata"]["uid"],
    }]);
}

/// Record a digest of the specification in an annotation.
fn hashed(mut object: Value) -> Value {
    let digest = Sha256::digest(object.to_string().as_bytes());
    let hash: String = digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let annotations = object["metadata"]["annotations"]
        .as_object()
        .cloned()
        .unwrap_or_else(Map::new);
    let mut annotations = annotations;
    annotations.insert(HASH_ANNOTATION.into(), json!(hash));
    object["metadata"]["annotations"] = Value::Object(annotations);
    object
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(arm: &str, version: Option<u32>) -> Key {
        Key {
            experiment: "price".into(),
            experiment_version: 2,
            arm: arm.into(),
            version,
        }
    }

    fn default() -> Value {
        json!({
            "metadata": {"name": "shop", "uid": "u-1"},
            "spec": {
                "replicas": 20,
                "selector": {"matchLabels": {"app": "shop"}},
                "template": {
                    "metadata": {
                        "labels": {"app": "shop"},
                        "annotations": {"prometheus.io/scrape": "true"},
                    },
                    "spec": {
                        "serviceAccountName": "shop",
                        "containers": [
                            {"name": "statespace-proxy", "image": "statespace/ssp:0.2.0"},
                            {
                                "name": "app",
                                "image": "ghcr.io/acme/shop:main",
                                "ports": [{"containerPort": 8000}],
                                "env": [
                                    {"name": "DATABASE_URL", "valueFrom": {"secretKeyRef": {"name": "db", "key": "url"}}},
                                    {"name": "CACHE_PREFIX", "value": "prod"},
                                ],
                            },
                        ],
                    },
                },
            },
        })
    }

    #[test]
    fn names_fit_kubernetes_and_differ_by_version() {
        let first = arm_name("shop", &key("lower-price", Some(1)));
        let second = arm_name("shop", &key("lower-price", Some(2)));
        assert!(first.starts_with("shop-price-lower-price-"));
        assert_ne!(first, second);
        let long = arm_name(&"x".repeat(80), &key("control", None));
        assert!(long.len() <= 63, "{long}");
        assert!(long.chars().all(|character| character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || character == '-'));
    }

    #[test]
    fn arms_copy_the_default_and_swap_its_image() {
        let default = default();
        let pod = &default["spec"]["template"]["spec"];
        let container = app_container(pod, None).unwrap();
        assert_eq!(container, 1);
        assert_eq!(container_port(&pod["containers"][1]), Some(8000));
        let key = key("lower-price", Some(3));
        let name = arm_name("shop", &key);
        let image = format!("ghcr.io/acme/shop@sha256:{}", "a".repeat(64));
        let environment = [
            ("CACHE_PREFIX".to_owned(), "variant".to_owned()),
            ("STATESPACE_VARIANT".to_owned(), "lower-price".to_owned()),
        ];
        let arm = arm_deployment(
            &default,
            container,
            "shop",
            &name,
            &key,
            &image,
            replicas(20, 0.05),
            &environment,
            Some(&default),
        )
        .unwrap();
        let spec = &arm["spec"]["template"]["spec"];
        // The sidecar stays with the default.
        assert_eq!(spec["containers"].as_array().unwrap().len(), 1);
        let app = &spec["containers"][0];
        assert_eq!(app["image"], image);
        assert_eq!(spec["serviceAccountName"], "shop");
        let env = app["env"].as_array().unwrap();
        assert!(env.iter().any(|variable| variable["name"] == "DATABASE_URL"
            && variable["valueFrom"]["secretKeyRef"]["name"] == "db"));
        assert!(env.contains(&json!({"name": "CACHE_PREFIX", "value": "variant"})));
        // The default's Service selects app=shop; arm pods do not carry it.
        let labels = &arm["spec"]["template"]["metadata"]["labels"];
        assert_eq!(labels["app"], Value::Null);
        assert_eq!(labels[ARM_LABEL], name.as_str());
        assert_eq!(arm["spec"]["replicas"], 1);
        assert_eq!(arm["metadata"]["ownerReferences"][0]["uid"], "u-1");
        assert_eq!(
            arm["spec"]["template"]["metadata"]["annotations"]["prometheus.io/scrape"],
            "true"
        );
        assert_eq!(key_of(&arm["spec"]["template"]), Some(key.clone()));
        let again = arm_deployment(
            &default,
            container,
            "shop",
            &name,
            &key,
            &image,
            1,
            &environment,
            Some(&default),
        )
        .unwrap();
        assert!(current(&arm, &again));
        let more = arm_deployment(
            &default,
            container,
            "shop",
            &name,
            &key,
            &image,
            2,
            &environment,
            Some(&default),
        )
        .unwrap();
        assert!(!current(&arm, &more));
    }

    #[test]
    fn replicas_follow_the_default_by_share() {
        assert_eq!(replicas(20, 0.05), 1);
        assert_eq!(replicas(20, 0.1), 2);
        assert_eq!(replicas(30, 0.05), 2);
        assert_eq!(replicas(0, 0.5), 1);
    }

    #[test]
    fn finds_the_named_container() {
        let pod = &default()["spec"]["template"]["spec"];
        assert_eq!(app_container(pod, Some("app")).unwrap(), 1);
        assert!(app_container(pod, Some("missing")).is_err());
        let only_proxy = json!({"containers": [{"name": PROXY_CONTAINER}]});
        assert!(app_container(&only_proxy, None).is_err());
    }
}
