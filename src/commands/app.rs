//! Apps: one codebase and deployment, with its own analysis schema.

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use super::print;
use crate::{
    api::{Api, segment},
    cli::AppCommand,
};

pub fn run(api: &Api, command: AppCommand) -> anyhow::Result<()> {
    match command {
        AppCommand::Create {
            name,
            output_retention,
        } => {
            let body = json!({ "name": name, "output_retention_days": output_retention });
            print(&api.send::<Value>(Method::POST, "/v1/apps", &body)?)
        }
        AppCommand::Update {
            name,
            output_retention,
        } => {
            let body = json!({ "output_retention_days": output_retention });
            print(&api.send::<Value>(Method::PATCH, &path(&name), &body)?)
        }
        AppCommand::List => print(&api.get::<Value>("/v1/apps")?),
        AppCommand::Show { name } => print(&api.get::<Value>(&path(&name))?),
        AppCommand::EraseSubject { name, subject } => {
            let erased: Value = api.send(
                Method::DELETE,
                &format!("{}/subjects/{}", path(&name), segment(&subject)),
                &json!({}),
            )?;
            print(&json!({ "app": name, "subject_id": subject, "erased": erased }))
        }
        AppCommand::Delete { name, yes } => {
            if !yes {
                bail!("pass --yes to delete {name} with its variants, experiments, and events");
            }
            api.delete(&path(&name))?;
            print(&json!({ "deleted": name }))
        }
    }
}

pub fn path(name: &str) -> String {
    format!("/v1/apps/{}", segment(name))
}
