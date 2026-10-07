//! Experiments: a draft of groups and weights, published by `start`.

use anyhow::{Context, bail};
use reqwest::Method;
use serde_json::{Map, Value, json};

use super::print;
use crate::{
    api::{Api, decode, segment},
    cli::{ExperimentCommand, Settings},
};

pub fn run(api: &Api, command: ExperimentCommand) -> anyhow::Result<()> {
    match command {
        ExperimentCommand::Create { name, settings } => {
            let mut body = settings_body(&settings)?;
            body.insert("name".into(), json!(name));
            print(&api.send::<Value>(Method::POST, "/v1/experiments", &Value::Object(body))?)
        }
        ExperimentCommand::Update {
            name,
            settings,
            no_eligibility,
        } => {
            let mut body = settings_body(&settings)?;
            if no_eligibility {
                body.insert("eligibility".into(), Value::Null);
            }
            if body.is_empty() {
                bail!("pass a setting to change, such as --group or --eligibility");
            }
            print(&api.send::<Value>(Method::PATCH, &path(&name), &Value::Object(body))?)
        }
        ExperimentCommand::List => print(&api.get::<Value>("/v1/experiments")?),
        ExperimentCommand::Show { name } => print(&api.get::<Value>(&path(&name))?),
        ExperimentCommand::Start { name, version } => {
            let body = json!({ "version": version });
            print(&api.send::<Value>(Method::POST, &format!("{}/start", path(&name)), &body)?)
        }
        ExperimentCommand::Stop { name } => {
            print(&api.send::<Value>(Method::POST, &format!("{}/stop", path(&name)), &json!({}))?)
        }
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
        ExperimentCommand::EraseSubject { name, subject } => {
            let erased: Value = decode(
                api.request(Method::DELETE, &format!("{}/subjects", path(&name)))?
                    .json(&json!({ "subject_id": subject }))
                    .send()?,
            )?;
            print(&json!({ "experiment": name, "subject_id": subject, "erased": erased }))
        }
        ExperimentCommand::Delete { name, yes } => {
            if !yes {
                bail!("pass --yes to delete {name} with all of its runs and outcomes");
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
    if !settings.groups.is_empty() {
        let mut groups = Map::new();
        for group in &settings.groups {
            let (name, weight) = parse_group(group)?;
            if groups.insert(name.into(), json!(weight)).is_some() {
                bail!("group {name} is listed twice");
            }
        }
        body.insert("groups".into(), Value::Object(groups));
    }
    for (key, value) in [
        ("eligibility", &settings.eligibility),
        ("assignment", &settings.assignment),
        ("description", &settings.description),
    ] {
        if let Some(value) = value {
            body.insert(key.into(), json!(value));
        }
    }
    Ok(body)
}

/// Parse `name=weight`, where the weight is greater than 0 and at most 1.
fn parse_group(value: &str) -> anyhow::Result<(&str, f64)> {
    let invalid = || format!("{value:?} must look like name=0.2");
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
    fn parses_group_weights() {
        assert_eq!(parse_group("bm25=0.25").unwrap(), ("bm25", 0.25));
        assert_eq!(parse_group("all=1").unwrap(), ("all", 1.0));
        for invalid in ["bm25", "bm25=0", "bm25=1.5", "=0.2", "bm25=20%"] {
            assert!(parse_group(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn sends_only_the_settings_that_were_passed() {
        let settings = Settings {
            groups: vec!["bm25=0.2".into()],
            eligibility: None,
            assignment: None,
            description: Some("Test BM25.".into()),
        };
        let body = settings_body(&settings).unwrap();
        assert_eq!(
            Value::Object(body),
            json!({"groups": {"bm25": 0.2}, "description": "Test BM25."})
        );
    }
}
