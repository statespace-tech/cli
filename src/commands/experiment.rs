//! Experiments: a draft of variants and weights, published by `start`.

use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::print;
use crate::{
    api::{Api, segment},
    cli::{ExperimentCommand, Settings},
};

pub fn run(api: &Api, command: ExperimentCommand) -> anyhow::Result<()> {
    match command {
        ExperimentCommand::Create {
            name,
            app,
            subject,
            settings,
        } => {
            let mut body = settings_body(&settings)?;
            body.insert("name".into(), json!(name));
            body.insert("app".into(), json!(app));
            if let Some(subject) = subject {
                body.insert("subject".into(), json!(subject));
            }
            print(&api.send::<Value>(Method::POST, "/v1/experiments", &Value::Object(body))?)
        }
        ExperimentCommand::Update {
            name,
            subject,
            settings,
            no_eligibility,
        } => {
            let mut body = settings_body(&settings)?;
            if let Some(subject) = subject {
                body.insert("subject".into(), json!(subject));
            }
            if no_eligibility {
                body.insert("eligibility".into(), Value::Null);
            }
            if body.keys().all(|key| key == "note") {
                bail!("pass a setting to change, such as --variant or --traffic");
            }
            print(&api.send::<Value>(Method::PATCH, &path(&name), &Value::Object(body))?)
        }
        ExperimentCommand::List { app } => {
            let query = app.map_or_else(String::new, |app| format!("?app={}", segment(&app)));
            print(&api.get::<Value>(&format!("/v1/experiments{query}"))?)
        }
        ExperimentCommand::Show { name } => print(&api.get::<Value>(&path(&name))?),
        ExperimentCommand::Start {
            name,
            version,
            note,
        } => {
            let body = json!({ "version": version, "note": note });
            print(&api.send::<Value>(Method::POST, &format!("{}/start", path(&name)), &body)?)
        }
        ExperimentCommand::Stop { name, note } => print(&api.send::<Value>(
            Method::POST,
            &format!("{}/stop", path(&name)),
            &json!({ "note": note }),
        )?),
        ExperimentCommand::Results {
            name,
            outcome,
            metric,
            version,
        } => {
            let mut query = format!("outcome={}", segment(&outcome));
            if let Some(metric) = metric {
                query.push_str(&format!("&metric={}", segment(&metric)));
            }
            if let Some(version) = version {
                query.push_str(&format!("&version={version}"));
            }
            print(&api.get::<Value>(&format!("{}/results?{query}", path(&name)))?)
        }
        ExperimentCommand::Delete { name, yes } => {
            if !yes {
                bail!("pass --yes to delete {name} with all of its versions and runs");
            }
            api.delete(&path(&name))?;
            print(&json!({ "deleted": name }))
        }
    }
}

fn path(name: &str) -> String {
    format!("/v1/experiments/{}", segment(name))
}

/// The settings that were passed, as request fields.
fn settings_body(settings: &Settings) -> anyhow::Result<Map<String, Value>> {
    let mut body = Map::new();
    if !settings.variants.is_empty() {
        let mut variants = Map::new();
        for variant in &settings.variants {
            let (name, weight) = parse_weight(variant)?;
            if variants.insert(name.into(), json!(weight)).is_some() {
                bail!("variant {name} is listed twice");
            }
        }
        body.insert("variants".into(), Value::Object(variants));
    }
    if let Some(traffic) = settings.traffic {
        if !(traffic > 0.0 && traffic <= 1.0) {
            bail!("--traffic must be greater than 0 and at most 1");
        }
        body.insert("traffic".into(), json!(traffic));
    }
    for (key, value) in [
        ("eligibility", &settings.eligibility),
        ("description", &settings.description),
        ("note", &settings.note),
    ] {
        if let Some(value) = value {
            body.insert(key.into(), json!(value));
        }
    }
    Ok(body)
}

/// Parse `name=weight`, where the weight is greater than 0 and at most 1.
fn parse_weight(value: &str) -> anyhow::Result<(&str, f64)> {
    let invalid = || format!("{value:?} must look like name=0.5");
    let (name, weight) = value.rsplit_once('=').with_context(invalid)?;
    let weight: f64 = weight.parse().with_context(invalid)?;
    if name.is_empty() || !(weight > 0.0 && weight <= 1.0) {
        bail!(
            "{}; the weight must be greater than 0 and at most 1",
            invalid()
        );
    }
    Ok((name, weight))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_variant_weights() {
        assert_eq!(parse_weight("fast=0.25").unwrap(), ("fast", 0.25));
        assert_eq!(parse_weight("all=1").unwrap(), ("all", 1.0));
        for invalid in ["fast", "fast=0", "fast=1.5", "=0.2", "fast=20%"] {
            assert!(parse_weight(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sends_only_the_settings_that_were_passed() {
        let settings = Settings {
            variants: vec!["fast=0.5".into()],
            traffic: Some(0.2),
            eligibility: None,
            description: None,
            note: Some("Faster checkout.".into()),
        };
        let body = settings_body(&settings).unwrap();
        assert_eq!(
            Value::Object(body),
            json!({"variants": {"fast": 0.5}, "traffic": 0.2, "note": "Faster checkout."})
        );
    }
}
