//! Tokens: credentials for programs, agents, and SQL, and sign-in sessions.

use anyhow::bail;
use reqwest::Method;
use serde_json::{Value, json};

use super::print;
use crate::{
    api::{Api, segment},
    cli::TokenCommand,
};

pub fn run(api: &Api, command: TokenCommand) -> anyhow::Result<()> {
    match command {
        TokenCommand::Create {
            name,
            preset,
            scopes,
            app,
            expires_in_days,
        } => {
            let scopes = match preset {
                Some(preset) => preset.scopes(),
                None => scopes.iter().map(String::as_str).collect(),
            };
            let body = json!({
                "name": name,
                "scopes": scopes,
                "app": app,
                "expires_in_days": expires_in_days,
            });
            print(&api.send::<Value>(Method::POST, "/v1/tokens", &body)?)
        }
        TokenCommand::List => print(&api.get::<Value>("/v1/tokens")?),
        TokenCommand::Revoke { id: Some(id), .. } => {
            api.delete(&format!("/v1/tokens/{}", segment(&id)))?;
            print(&json!({ "revoked": id }))
        }
        TokenCommand::Revoke { id: None, yes, .. } => {
            if !yes {
                bail!("pass --yes to revoke every token and session of the account");
            }
            print(&api.send::<Value>(Method::POST, "/v1/tokens/revoke-all", &json!({}))?)
        }
    }
}
