//! Experiments: a draft of arms and their shares, published by `start`.
//!
//! People give each arm's share of all the app's subjects, as in
//! `--split lower-price=0.05`. The API stores the experiment's share of the
//! app and each variant's share of the experiment; this module converts
//! between the two.

use std::collections::BTreeMap;

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
            if settings.split.is_empty() {
                bail!("pass --split with each variant's share, such as --split lower-price=0.05");
            }
            let mut body = settings_body(&settings)?;
            body.insert("name".into(), json!(name));
            body.insert("app".into(), json!(app));
            if let Some(subject) = subject {
                body.insert("subject".into(), json!(subject));
            }
            show(api.send(Method::POST, "/v1/experiments", &Value::Object(body))?)
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
                bail!("pass a setting to change, such as --split or --eligibility");
            }
            show(api.send(Method::PATCH, &path(&name), &Value::Object(body))?)
        }
        ExperimentCommand::List { app } => {
            let query = app.map_or_else(String::new, |app| format!("?app={}", segment(&app)));
            let experiments: Vec<Value> = api.get(&format!("/v1/experiments{query}"))?;
            print(&experiments.into_iter().map(with_split).collect::<Vec<_>>())
        }
        ExperimentCommand::Show { name } => show(api.get(&path(&name))?),
        ExperimentCommand::Start {
            name,
            version,
            note,
        } => {
            let body = json!({ "version": version, "note": note });
            show(api.send(Method::POST, &format!("{}/start", path(&name)), &body)?)
        }
        ExperimentCommand::Stop { name, note } => show(api.send(
            Method::POST,
            &format!("{}/stop", path(&name)),
            &json!({ "note": note }),
        )?),
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

fn show(experiment: Value) -> anyhow::Result<()> {
    print(&with_split(experiment))
}

/// Add `split`: each arm's share of all the app's subjects.
fn with_split(mut experiment: Value) -> Value {
    let traffic = experiment["traffic"].as_f64().unwrap_or(0.0);
    let mut split = Map::new();
    if let Some(variants) = experiment["variants"].as_object() {
        for (name, weight) in variants {
            split.insert(
                name.clone(),
                json!(round(traffic * weight.as_f64().unwrap_or(0.0))),
            );
        }
    }
    let control = experiment["control"].as_f64().unwrap_or(0.0);
    split.insert("control".into(), json!(round(traffic * control)));
    experiment["split"] = Value::Object(split);
    experiment
}

/// Drop floating-point noise from a share.
fn round(share: f64) -> f64 {
    (share * 1e9).round() / 1e9
}

/// The settings that were passed, as request fields.
fn settings_body(settings: &Settings) -> anyhow::Result<Map<String, Value>> {
    let mut body = Map::new();
    if !settings.split.is_empty() {
        let (traffic, weights) = split(&settings.split)?;
        body.insert("traffic".into(), json!(traffic));
        body.insert("variants".into(), json!(weights));
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

/// Turn each arm's share of the app's subjects into the experiment's share
/// of the app and each variant's share of the experiment. Control gets the
/// largest variant's share unless `control=SHARE` sets it.
fn split(arms: &[String]) -> anyhow::Result<(f64, BTreeMap<String, f64>)> {
    let mut shares = BTreeMap::new();
    for arm in arms {
        let (name, share) = parse_share(arm)?;
        if shares.insert(name.to_owned(), share).is_some() {
            bail!("{name} is listed twice");
        }
    }
    let control = shares.remove("control");
    if shares.is_empty() {
        bail!("--split needs at least one variant besides control");
    }
    let control = control.unwrap_or_else(|| shares.values().copied().fold(0.0, f64::max));
    let traffic = control + shares.values().sum::<f64>();
    if traffic > 1.0 + 1e-9 {
        bail!(
            "the arms hold {} of the app's subjects, control included, which exceeds 1",
            round(traffic)
        );
    }
    let traffic = traffic.min(1.0);
    let weights = shares
        .into_iter()
        .map(|(name, share)| (name, share / traffic))
        .collect();
    Ok((traffic, weights))
}

/// Parse `name=share`, where the share is greater than 0 and at most 1.
fn parse_share(value: &str) -> anyhow::Result<(&str, f64)> {
    let invalid = || format!("{value:?} must look like name=0.05");
    let (name, share) = value.rsplit_once('=').with_context(invalid)?;
    let share: f64 = share.parse().with_context(invalid)?;
    if name.is_empty() || !(share > 0.0 && share <= 1.0) {
        bail!(
            "{}; the share must be greater than 0 and at most 1",
            invalid()
        );
    }
    Ok((name, share))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arms(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parses_shares() {
        assert_eq!(parse_share("fast=0.25").unwrap(), ("fast", 0.25));
        assert_eq!(parse_share("all=1").unwrap(), ("all", 1.0));
        for invalid in ["fast", "fast=0", "fast=1.5", "=0.2", "fast=20%"] {
            assert!(parse_share(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn control_matches_the_largest_variant() {
        let (traffic, weights) = split(&arms(&["lower-price=0.05"])).unwrap();
        assert!((traffic - 0.1).abs() < 1e-12);
        assert!((weights["lower-price"] - 0.5).abs() < 1e-12);

        let (traffic, weights) = split(&arms(&["a=0.05", "b=0.02"])).unwrap();
        assert!((traffic - 0.12).abs() < 1e-12);
        assert!((weights["a"] - 0.05 / 0.12).abs() < 1e-12);
        assert!((weights["b"] - 0.02 / 0.12).abs() < 1e-12);

        let (traffic, weights) = split(&arms(&["a=0.1", "control=0.3"])).unwrap();
        assert!((traffic - 0.4).abs() < 1e-12);
        assert!((weights["a"] - 0.25).abs() < 1e-12);
    }

    #[test]
    fn rejects_splits_beyond_the_app() {
        assert!(split(&arms(&["a=0.6"])).is_err());
        assert!(split(&arms(&["control=0.1"])).is_err());
        assert!(split(&arms(&["a=0.1", "a=0.2"])).is_err());
        assert!(split(&arms(&["a=0.5"])).is_ok());
    }

    #[test]
    fn shows_shares_of_the_app() {
        let shown = with_split(json!({
            "traffic": 0.1, "variants": {"lower-price": 0.5}, "control": 0.5
        }));
        assert_eq!(
            shown["split"],
            json!({"lower-price": 0.05, "control": 0.05})
        );
    }

    #[test]
    fn sends_only_the_settings_that_were_passed() {
        let settings = Settings {
            split: arms(&["fast=0.25"]),
            eligibility: None,
            description: None,
            note: Some("Faster checkout.".into()),
        };
        let body = settings_body(&settings).unwrap();
        assert_eq!(
            Value::Object(body),
            json!({"traffic": 0.5, "variants": {"fast": 0.5}, "note": "Faster checkout."})
        );
    }
}
