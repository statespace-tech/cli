//! Reusable groups of parameter values.

use reqwest::Method;
use serde_json::{Value, json};

use super::print;
use crate::{
    api::{Api, segment},
    cli::GroupCommand,
    params,
};

pub fn run(api: &Api, command: GroupCommand) -> anyhow::Result<()> {
    match command {
        GroupCommand::Create { name, parameters } => {
            let parameters = params::resolve(api, &parameters)?;
            if parameters.values().any(Value::is_null) {
                anyhow::bail!("a new group cannot remove parameters");
            }
            let body = json!({ "name": name, "parameters": parameters });
            print(&api.send::<Value>(Method::POST, "/v1/groups", &body)?)
        }
        GroupCommand::Update { name, parameters } => {
            let body = json!({ "parameters": params::resolve(api, &parameters)? });
            print(&api.send::<Value>(Method::PATCH, &path(&name), &body)?)
        }
        GroupCommand::List => print(&api.get::<Value>("/v1/groups")?),
        GroupCommand::Show { name } => print(&api.get::<Value>(&path(&name))?),
        GroupCommand::Delete { name } => {
            api.delete(&path(&name))?;
            print(&json!({ "deleted": name }))
        }
    }
}

fn path(name: &str) -> String {
    format!("/v1/groups/{}", segment(name))
}
